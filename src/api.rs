use std::cmp::min;
use std::env;
use std::future::Future;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use geo::{Distance, Haversine, Point};
use geo_types::Coord;
use indicatif::{ProgressBar, ProgressStyle};
use polyline;
use rand::RngExt;
use reqwest::{self, Client};
use serde_json;
use tokio::task;
use tokio::time::{self, sleep, Duration};

use crate::utils::PROXIMITY_THRESHOLD_METERS;
use crate::models::{
    Hydrant, Location, LocationResponse, MapLine, MapMarker, MapRequest, ORSResponse, OSRMResponse,
    HydrantWithRoute, RouteDetails,
};

/// Der maximale Suchradius in Metern für Hydranten.
const MAX_RADIUS: u32 = 1_500;
/// Die Genauigkeit, mit der Polylines kodiert werden sollen.
const POLYLINE_PRECISION: u32 = 6;
/// Die Standardanzahl an zurückzugebenden Hydranten, falls kein Limit angegeben wurde.
const DEFAULT_HYDRANT_LIMIT: usize = 5;

const OVERPASS_ENDPOINTS: &[&str] = &[
    "https://overpass-api.de/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
    "https://maps.mail.ru/osm/tools/overpass/api/interpreter",
];

// --- Hydranten-Suche (Overpass) ---

/// Ruft Hydranten im angegebenen Radius um die übergebene Position von der Overpass API ab.
///
/// Führt interne Retries bei transienten Fehlern durch. Verdoppelt den Suchradius (`initial_radius_m`)
/// bei jedem erfolglosen Versuch, bis mindestens 5 Hydranten gefunden oder der `MAX_RADIUS` erreicht wird.
///
/// # Arguments
///
/// * `location` - Die Startposition.
/// * `initial_radius_m` - Der anfängliche Suchradius in Metern (z.B. 500m).
/// * `client` - Ein geteilter reqwest Client.
///
/// # Returns
///
/// Eine `Vec<Hydrant>` der gefundenen Hydranten.
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn die Overpass API nach allen Retries einen nicht erfolgreichen Statuscode zurückgibt.
pub async fn find_hydrants(location: &Location, initial_radius_m: u32, client: &Client) -> Result<Vec<Hydrant>> {
    let lat = location.latitude;
    let lon = location.longitude;
    let mut radius_m = initial_radius_m;
    let mut radius_attempt = 0;

    loop {
        let query = format!(
            r#"[out:json][timeout:20];
            (
            node(around:{}, {}, {})[emergency=fire_hydrant];
            );
            out tags center;"#,
            radius_m, lat, lon
        );

        let mut last_err = None;
        let mut response_opt = None;

        for &url in OVERPASS_ENDPOINTS {
            let res = retry_on_failure(
                || {
                    let client_clone = client.clone();
                    let query_clone = query.clone();

                    async move {
                        let resp = client_clone
                            .post(url)
                            .body(query_clone)
                            .send()
                            .await?;
                        resp.error_for_status().map_err(anyhow::Error::from)
                    }
                },
                3,
                Duration::from_secs(2),
            )
                .await;

            match res {
                Ok(r) => {
                    response_opt = Some(r);
                    break;
                }
                Err(e) => {
                    eprintln!("[!] Endpoint {} fehlgeschlagen, wechsle Mirror...", url);
                    last_err = Some(e);
                }
            }
        }

        let response = match response_opt {
            Some(r) => r,
            None => return Err(last_err.unwrap_or_else(|| anyhow::anyhow!("[X] Alle Overpass-Endpoints fehlgeschlagen."))),
        };

        let response_body = response.json::<LocationResponse>().await?;
        let hydrants = response_body.elements;

        if hydrants.len() >= 5 || radius_m >= MAX_RADIUS {
            return Ok(hydrants);
        }

        radius_attempt += 1;
        radius_m = min(initial_radius_m * (2_u32.pow(radius_attempt)), MAX_RADIUS);
        sleep(Duration::from_millis(1300)).await;
    }
}

/// Ruft die nächstgelegenen Hydranten (Luftlinie) ab.
///
/// Die Hydranten werden zuerst über `find_hydrants` gesucht (aktuell mit festem Startradius von 500m),
/// dann nach Haversine-Distanz sortiert und auf die gewünschte Anzahl begrenzt.
///
/// # Arguments
///
/// * `location` - Die Startposition des Benutzers.
/// * `amount` - Die maximale Anzahl der zurückzugebenden Hydranten (Standard: 5).
/// * `client` - Ein geteilter reqwest Client.
///
/// # Returns
///
/// Eine `Vec<Hydrant>` der nächstgelegenen Hydranten, sortiert nach Luftlinie.
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn keine Hydranten im Suchradius gefunden wurden.
pub async fn get_nearest_hydrants(
    location: &Location,
    amount: Option<usize>,
    client: &Client
) -> Result<Vec<Hydrant>> {
    let radius_m = 500;

    let hydrant_locations = find_hydrants(location, radius_m, client)
        .await
        .context("Fehler beim Abruf der Hydranten")?;

    if hydrant_locations.is_empty() {
        anyhow::bail!(
            "[-] Keine Hydranten in der Nähe gefunden (Radius: {}m).",
            radius_m
        );
    }

    let user_location_point = Point::new(location.longitude, location.latitude);

    let mut nearest_hydrants: Vec<(Hydrant, f64)> = hydrant_locations
        .into_iter()
        .map(|hydrant| {
            let hydrant_point = Point::new(hydrant.longitude, hydrant.latitude);
            let distance = Haversine.distance(user_location_point, hydrant_point);
            (hydrant, distance)
        })
        .collect();

    nearest_hydrants.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let max_len = amount.unwrap_or(DEFAULT_HYDRANT_LIMIT);
    let final_coords: Vec<Hydrant> = nearest_hydrants
        .into_iter()
        .take(max_len)
        .map(|(coord, _distance)| coord)
        .collect();

    Ok(final_coords)
}

// --- Routenberechnung (OSRM / ORS Fallback) ---

/// Berechnet die Route zwischen Start und Ziel mithilfe der OSRM (Open Source Routing Machine) API.
///
/// Führt interne Retries bei transienten Fehlern durch. Bei permanenten Fehlern,
/// HTTP-Fehler-Statuscodes oder ungültiger/0m Route wird automatisch auf `fetch_route_ors` zurückgefallen.
///
/// # Arguments
///
/// * `client` - Ein geteilter reqwest Client.
/// * `start` - Die Startposition.
/// * `destination` - Die Zielposition (Hydrant).
///
/// # Returns
///
/// Die `RouteDetails` der berechneten Route, oder ein Fallback auf ORS.
pub async fn fetch_route_osrm(
    client: &Client,
    start: &Location,
    destination: &Hydrant,
) -> Result<RouteDetails> {
    let url = format!(
        "https://router.project-osrm.org/route/v1/foot/{},{};{},{}?steps=false&snapping=any&overview=full&geometries=polyline6",
        start.longitude, start.latitude, destination.longitude, destination.latitude
    );

    let response = retry_on_failure(
        || {
            let client_clone = client.clone();
            let url_clone = url.clone();

            async move {
                client_clone
                    .get(&url_clone)
                    .send()
                    .await
                    .map_err(anyhow::Error::from)
            }
        },
        2, // Max. 2 Versuche (1. Versuch + 1 Retry)
        Duration::from_secs(2),
    )
        .await;

    let response = match response {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[!] OSRM-Netzwerkfehler (nach Retries): {}. Versuche ORS...", e);
            // Fallback auf ORS
            return fetch_route_ors(client, start, destination).await;
        }
    };

    if !response.status().is_success() {
        eprintln!(
            "[!] OSRM-API-Fehler (Status: {}). Versuche ORS...",
            response.status()
        );
        // Fallback auf ORS
        return fetch_route_ors(client, start, destination).await;
    }

    let response_body = response.json::<OSRMResponse>().await?;

    if let Some(route) = response_body.routes.into_iter().next() {
        if let Some(leg) = route.legs.into_iter().next() {
            if leg.distance == 0.0 {
                // 0m-Route Fallback
                eprintln!("[!] OSRM lieferte 0m-Route. Versuche ORS...");
                return fetch_route_ors(client, start, destination).await;
            }
            return Ok(RouteDetails {
                distance_meters: leg.distance,
                geometry_polyline: route.geometry,
            });
        }
    }

    // Fallback, wenn keine Route im Body gefunden wurde
    eprintln!("[!] OSRM lieferte keine Route im Body. Versuche ORS...",);
    fetch_route_ors(client, start, destination).await
}

/// Berechnet die Route zwischen Start und Ziel mithilfe der OpenRouteService (ORS) API.
///
/// ORS dient als Fallback für OSRM und erfordert einen API-Schlüssel (`ORS_API_KEY`).
/// Führt interne Retries bei transienten Fehlern durch. Meldet ORS eine 0m-Route,
/// wird die Haversine-Distanz (Luftlinie) als Fallback zurückgegeben (falls > 10m).
/// Wichtig: Bei Fallback auf Luftlinie wird `geometry_polyline` leer zurückgegeben.
///
/// # Arguments
///
/// * `client` - Ein geteilter reqwest Client.
/// * `start` - Die Startposition.
/// * `destination` - Die Zielposition (Hydrant).
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn die `ORS_API_KEY` Umgebungsvariable fehlt oder
/// die API nach allen Retries einen Fehlerstatus zurückgibt.
pub async fn fetch_route_ors(
    client: &Client,
    start: &Location,
    destination: &Hydrant,
) -> Result<RouteDetails> {
    let api_key =
        env::var("ORS_API_KEY").context("[X] ORS_API_KEY Umgebungsvariable muss gesetzt sein. Routing nicht möglich.")?;

    let url = format!(
        "https://api.openrouteservice.org/v2/directions/foot-walking?api_key={}&start={},{}&end={},{}&snapping=any",
        api_key, start.longitude, start.latitude, destination.longitude, destination.latitude
    );

    let response = retry_on_failure(
        || {
            let client_clone = client.clone();
            let url_clone = url.clone();

            async move {
                client_clone
                    .get(&url_clone)
                    .send()
                    .await
                    .map_err(anyhow::Error::from)
            }
        },
        2, // Max. 2 Versuche (1. Versuch + 1 Retry)
        Duration::from_secs(2),
    )
        .await;

    // Nun behandeln wir das finale Ergebnis:
    let response = match response {
        Ok(r) => r,
        Err(e) => {
            anyhow::bail!("[X] ORS-API-Aufruf fehlgeschlagen: {}", e);
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .context("ORS: Kein Antwort-Body verfügbar")?;
        anyhow::bail!("[X] ORS-API-Fehler (Status: {}). Antwort: {}", status, body);
    }

    let body_text = response.text().await?;

    let response_body: ORSResponse = match serde_json::from_str(&body_text) {
        Ok(body) => body,
        Err(e) => {
            anyhow::bail!(
                "[X] ORS-JSON-Decode-Fehler: {}. Ungültige JSON-Antwort (Ausschnitt): {}",
                e,
                &body_text[..body_text.len().min(100)]
            );
        }
    };

    if let Some(feature) = response_body.features.into_iter().next() {
        let route_distance = feature.properties.summary.distance;
        let coords = feature.geometry.coordinates;

        let geo_coords: Vec<Coord> = coords
            .into_iter()
            .map(|c| Coord { x: c[0], y: c[1] })
            .collect();

        let geometry_polyline = polyline::encode_coordinates(geo_coords, POLYLINE_PRECISION)?;

        // Fallback-Logik, wenn ORS 0m meldet
        if route_distance == 0.0 {
            let start_point = Point::new(start.longitude, start.latitude);
            let dest_point = Point::new(destination.longitude, destination.latitude);
            let haversine_dist = Haversine.distance(start_point, dest_point);

            if haversine_dist > 5.0 {
                eprintln!(
                    "[!] Fallback: ORS-Route war 0m. Gebe Luftlinie ({:.2}m) zurück.",
                    haversine_dist
                );
                return Ok(RouteDetails {
                    distance_meters: haversine_dist,
                    geometry_polyline: String::new(),
                });
            }
        }

        return Ok(RouteDetails {
            distance_meters: route_distance,
            geometry_polyline,
        });
    }

    anyhow::bail!("[X] Keine Route (Feature) in der ORS-Antwort gefunden.");
}

// --- Ranking / Routen-Aggregator ---

/// Ermittelt den am besten bewerteten Hydranten unter den Kandidaten.
///
/// Parallelisierung: Die Routenberechnung für jeden Kandidaten erfolgt asynchron und parallel
/// mithilfe von `tokio::task::spawn`.
///
/// Optimierung: Wenn der Hydrant, der per Luftlinie am nächsten liegt,
/// bereits innerhalb des `PROXIMITY_THRESHOLD_METERS` (5m) liegt, wird sofort diese Position
/// ohne Routenberechnung als gültiger, gerankter Hydrant hinzugefügt (diese wird nicht von der Task-Timeout-Logik beeinflusst).
///
/// Timeout: Wenn die Berechnung aller Routen länger als 5 Sekunden dauert, wird mit den bis dahin
/// gefundenen Routen fortgefahren, sofern mindestens 3 Ergebnisse vorliegen. Andernfalls wird ein Fehler zurückgegeben.
///
/// # Arguments
///
/// * `user_location` - Die Startposition des Benutzers.
/// * `candidate_hydrants` - Eine Liste von Hydranten, die in Betracht gezogen werden sollen (typischerweise die nach Luftlinie nächsten).
/// * `client` - Ein geteilter reqwest Client.
///
/// # Returns
///
/// Eine sortierte `Vec<HydrantWithRoute>` mit dem besten (kürzester Weg) an erster Stelle.
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn keine gültige Route zu einem der Hydranten gefunden werden konnte oder das Timeout mit zu wenigen Ergebnissen ($<3$) überschritten wird.
pub async fn get_closest_by_route(
    user_location: &Location,
    candidate_hydrants: &[Hydrant],
    client: &Client
) -> Result<Vec<HydrantWithRoute>> {
    if candidate_hydrants.is_empty() {
        anyhow::bail!("[X] Keine Hydranten-Kandidaten zur Routenberechnung vorhanden.");
    }

    let user_location_point = Point::new(user_location.longitude, user_location.latitude);

    let spinner_style = ProgressStyle::with_template("{spinner:.cyan.bold} {msg}")?;

    let pb = ProgressBar::new(candidate_hydrants.len() as u64);
    pb.set_style(spinner_style);
    pb.set_message("Berechne Routen zu Hydranten...");
    pb.enable_steady_tick(Duration::from_millis(100));

    let all_ranked_hydrants_arc: Arc<Mutex<Vec<HydrantWithRoute>>> = Arc::new(Mutex::new(Vec::new()));

    let mut join_handles = Vec::new();

    let user_location_owned = user_location.clone();

    for hydrant in candidate_hydrants {
        let pb_clone = pb.clone();

        let hydrant_point = Point::new(hydrant.longitude, hydrant.latitude);

        let haversine_dist = Haversine.distance(user_location_point, hydrant_point);

        if haversine_dist <= PROXIMITY_THRESHOLD_METERS {
            let candidate = HydrantWithRoute::new(
                hydrant.clone(),
                RouteDetails {
                    distance_meters: haversine_dist,
                    geometry_polyline: String::new(),
                },
            );

            all_ranked_hydrants_arc.lock()
                .expect("Mutex der Ergebnisse ist vergiftet (Poisoned). Ein anderer Thread ist abgestürzt.")
                .push(candidate);
            pb_clone.inc(1);
            continue;
        }

        let client_clone = client.clone();
        let hydrant_clone = hydrant.clone();
        let user_location_task = user_location_owned.clone();
        let all_ranked_hydrants_clone = Arc::clone(&all_ranked_hydrants_arc);

        let handle = task::spawn(async move {
            match fetch_route_osrm(&client_clone, &user_location_task, &hydrant_clone).await {
                Ok(details) => {
                    let candidate = HydrantWithRoute::new(hydrant_clone, details);
                    all_ranked_hydrants_clone.lock().unwrap().push(candidate);
                }
                Err(e) => {
                    pb_clone.println(format!("[!] Route zu Hydrant übersprungen: {}", e));
                }
            }
            pb_clone.inc(1);
        });

        join_handles.push(handle);
    }

    let timeout_duration = Duration::from_secs(5);

    match time::timeout(timeout_duration, async {
        for handle in join_handles {
            handle.await?;
        }
        Ok(()) as Result<()>
    }).await {
        Err(e) => {
            pb.println(format!("[!] Timeout nach {}s. Routenberechnung nicht komplett.", timeout_duration.as_secs()));
            let num_results = all_ranked_hydrants_arc.lock()
                .expect("Mutex der Ranglisten-Hydranten ist vergiftet beim Abrufen der Anzahl (len).")
                .len();

            if num_results < 3 {
                pb.finish();
                return Err(anyhow::anyhow!("Route-Berechnung konnte nicht rechtzeitig mindestens 3 Hydranten liefern (nur {})", num_results).context(e));
            }

        }
        Ok(Err(e)) => {
            pb.finish();
            return Err(e);
        }
        Ok(Ok(_)) => {

        }
    };

    pb.finish();

    let mut sorted_hydrants = all_ranked_hydrants_arc.lock()
        .expect("Mutex der Ranglisten-Hydranten ist vergiftet beim Entleeren (drain) der Ergebnisse.")
        .drain(..)
        .collect::<Vec<HydrantWithRoute>>();

    if sorted_hydrants.is_empty() {
        anyhow::bail!("[X] Keine Routen zu Hydranten gefunden, auch nicht per Fallback.");
    }

    sorted_hydrants.sort_by(|a, b| a.route_details.distance_meters.partial_cmp(&b.route_details.distance_meters).unwrap_or(std::cmp::Ordering::Equal));

    Ok(sorted_hydrants)
}

// --- Hilfsfunktionen (Retry) ---

/// Führt die übergebene asynchrone Funktion bis zu `max_retries` Mal aus,
/// wenn ein transienter Fehler auftritt.
///
/// Transiente Fehler sind: HTTP 5xx Statuscodes, HTTP 429 (Too Many Requests),
/// Netzwerk-Timeouts und Verbindungsfehler.
///
/// # Arguments
///
/// * `f` - Die asynchrone Funktion, die ausgeführt werden soll.
/// * `max_retries` - Die maximale Anzahl der Versuche (1. Versuch + N Retries).
/// * `base_delay` - Die Basis-Wartezeit zwischen den Versuchen, die exponentiell erhöht wird (Exponential Backoff).
///
/// # Returns
///
/// Das Ergebnis von `f`, oder ein Fehler (`anyhow::Error`), wenn alle Versuche fehlschlagen.
pub async fn retry_on_failure<T, F, Fut>(
    f: F,
    max_retries: u32,
    base_delay: Duration,
) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut attempt = 1;

    loop {
        match f().await {
            Ok(result) => return Ok(result),

            Err(e) => {
                if !is_transient_error(&e) {
                    return Err(e).context("API-Aufruf fehlgeschlagen: Permanenter Fehler.");
                }

                if attempt >= max_retries {
                    anyhow::bail!(
                        "[X] API-Aufruf nach {} Versuchen fehlgeschlagen. Letzter Fehler: {}",
                        max_retries,
                        e
                    );
                }
                let jitter = Duration::from_millis(rand::rng().random_range(0..500));
                let delay = base_delay * (2_i32.pow(attempt) as u32) + jitter;
                println!(
                    "[!] Versuch {}/{}: API fehlgeschlagen. Warte {:.1}s...",
                    attempt,
                    max_retries,
                    delay.as_secs_f32()
                );
                sleep(delay).await;

                attempt += 1;
            }
        }
    }
}

/// Überprüft, ob ein `anyhow::Error` einen transienten Fehler umschließt,
/// der durch einen Retry behoben werden könnte.
///
/// Transiente Fehler: HTTP 5xx, HTTP 429, Timeout, Verbindungsfehler.
///
/// # Arguments
///
/// * `e` - Der `anyhow::Error`, der geprüft werden soll.
///
/// # Returns
///
/// `true`, wenn der Fehler transient ist, andernfalls `false`.
fn is_transient_error(e: &anyhow::Error) -> bool {
    if let Some(reqwest_error) = e.downcast_ref::<reqwest::Error>() {
        if let Some(status) = reqwest_error.status() {
            if status ==  reqwest::StatusCode::GATEWAY_TIMEOUT ||
                status == reqwest::StatusCode::SERVICE_UNAVAILABLE ||
                status == reqwest::StatusCode::TOO_MANY_REQUESTS ||
                status == reqwest::StatusCode::INTERNAL_SERVER_ERROR ||
                status == reqwest::StatusCode::REQUEST_TIMEOUT ||
                status.is_server_error() {
                return true;
            }
        }

        if reqwest_error.is_timeout() || reqwest_error.is_connect() {
            return true;
        }

        if reqwest_error.is_request() {
            return true;
        }
    }

    false
}