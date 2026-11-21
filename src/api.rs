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
use tokio::time::{self, Duration, sleep};

use crate::models::{
    Hydrant, HydrantWithRoute, Location, LocationResponse, ORSResponse, OSRMResponse,
    RouteDetails,
};
use crate::utils::PROXIMITY_THRESHOLD_METERS;

/// Maximaler Suchradius in Metern für die Hydrantensuche über Overpass.
const MAX_RADIUS: u32 = 1_500;

/// Genauigkeit (Dezimalstellen), mit der Polylines kodiert und dekodiert werden.
pub const POLYLINE_PRECISION: u32 = 6;

/// Standardanzahl zurückzugebender Hydranten, falls kein Limit angegeben wurde.
const DEFAULT_HYDRANT_LIMIT: usize = 5;

/// Liste alternativer Overpass-API-Endpoints für automatisches Failover.
const OVERPASS_ENDPOINTS: &[&str] = &[
    "https://overpass-api.de/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
    "https://maps.mail.ru/osm/tools/overpass/api/interpreter",
];

// --- Hydranten-Suche (Overpass) ---

/// Ruft Hydranten im angegebenen Radius um die übergebene Position von der Overpass API ab.
///
/// Wechselt bei Fehlern durch die konfigurierten [`OVERPASS_ENDPOINTS`] und führt interne Retries durch.
/// Verdoppelt den Suchradius bei weniger als 5 Treffern schrittweise bis maximal [`MAX_RADIUS`].
///
/// # Arguments
///
/// * `location` - Die Startposition für die Umkreissuche.
/// * `initial_radius_m` - Der anfängliche Suchradius in Metern (z. B. `500`).
/// * `client` - Ein geteilter HTTP-Client.
///
/// # Returns
///
/// Ein `Vec<Hydrant>` mit allen im finalen Radius gefundenen Hydranten.
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn alle Overpass-Endpoints fehlschlagen oder die Antwort
/// nicht als JSON deserialisiert werden kann.
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

/// Ruft die nach Luftlinie nächstgelegenen Hydranten ab.
///
/// Sucht Hydranten über [`find_hydrants`] mit einem Startradius von 500 Metern,
/// sortiert sie aufsteigend nach Haversine-Distanz und begrenzt das Ergebnis.
///
/// # Arguments
///
/// * `location` - Die Startposition des Benutzers.
/// * `amount` - Optionale maximale Anzahl zurückzugebender Hydranten (Standard: [`DEFAULT_HYDRANT_LIMIT`]).
/// * `client` - Ein geteilter HTTP-Client.
///
/// # Returns
///
/// Ein nach Luftlinie sortierter `Vec<Hydrant>`.
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn der Overpass-Abruf fehlschlägt oder keine Hydranten gefunden wurden.
pub async fn get_nearest_hydrants(
    location: &Location,
    amount: Option<usize>,
    client: &Client,
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

/// Berechnet eine Fußgängerroute zwischen Start und Ziel mithilfe der OSRM API.
///
/// Führt bei transienten Fehlern Retries durch und wechselt bei Netzwerkfehlern,
/// ungültigen HTTP-Statuscodes, leeren Routen oder 0m-Distanzen automatisch auf [`fetch_route_ors`].
///
/// # Arguments
///
/// * `client` - Ein geteilter HTTP-Client.
/// * `start` - Die Startposition.
/// * `destination` - Der Zielhydrant.
///
/// # Returns
///
/// Die berechneten [`RouteDetails`] von OSRM oder dem ORS-Fallback.
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn die OSRM-Antwort nicht geparst werden kann oder
/// der Fallback auf [`fetch_route_ors`] fehlschlägt.
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
        2,
        Duration::from_secs(2),
    )
        .await;

    let response = match response {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[!] OSRM-Netzwerkfehler (nach Retries): {}. Versuche ORS...", e);
            return fetch_route_ors(client, start, destination).await;
        }
    };

    if !response.status().is_success() {
        eprintln!(
            "[!] OSRM-API-Fehler (Status: {}). Versuche ORS...",
            response.status()
        );
        return fetch_route_ors(client, start, destination).await;
    }

    let response_body = response.json::<OSRMResponse>().await?;

    if let Some(route) = response_body.routes.into_iter().next() {
        if let Some(leg) = route.legs.into_iter().next() {
            if leg.distance == 0.0 {
                eprintln!("[!] OSRM lieferte 0m-Route. Versuche ORS...");
                return fetch_route_ors(client, start, destination).await;
            }
            return Ok(RouteDetails {
                distance_meters: leg.distance,
                geometry_polyline: route.geometry,
            });
        }
    }

    eprintln!("[!] OSRM lieferte keine Route im Body. Versuche ORS...");
    fetch_route_ors(client, start, destination).await
}

/// Berechnet eine Fußgängerroute zwischen Start und Ziel mithilfe der OpenRouteService (ORS) API.
///
/// Dient als Fallback für [`fetch_route_osrm`] und benötigt die Umgebungsvariable `ORS_API_KEY`.
/// Meldet ORS eine 0m-Route bei einer Luftlinie von über 5 Metern, wird die Haversine-Distanz
/// mit leerer Polyline zurückgegeben.
///
/// # Arguments
///
/// * `client` - Ein geteilter HTTP-Client.
/// * `start` - Die Startposition.
/// * `destination` - Der Zielhydrant.
///
/// # Returns
///
/// Die berechneten [`RouteDetails`] inklusive Polyline6-kodierter Geometrie.
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn `ORS_API_KEY` fehlt, die API nach Retries einen Fehlerstatus
/// liefert, die JSON-Antwort ungültig ist oder kein Routen-Feature enthalten ist.
pub async fn fetch_route_ors(
    client: &Client,
    start: &Location,
    destination: &Hydrant,
) -> Result<RouteDetails> {
    let api_key = env::var("ORS_API_KEY")
        .context("[X] ORS_API_KEY Umgebungsvariable muss gesetzt sein. Routing nicht möglich.")?;

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
        2,
        Duration::from_secs(2),
    )
        .await;

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

/// Berechnet parallel die Fußweg-Routen zu allen Hydranten-Kandidaten und sortiert sie nach Distanz.
///
/// Kandidaten innerhalb von [`PROXIMITY_THRESHOLD_METERS`] werden direkt ohne API-Anfrage
/// mit Luftliniendistanz übernommen. Alle übrigen Routen werden parallel in Tokio-Tasks mit einem
/// Gesamt-Timeout von 5 Sekunden berechnet.
///
/// # Arguments
///
/// * `user_location` - Die Startposition des Benutzers.
/// * `candidate_hydrants` - Liste der zu bewertenden Hydranten-Kandidaten.
/// * `client` - Ein geteilter HTTP-Client.
///
/// # Returns
///
/// Ein nach Routendistanz aufsteigend sortierter `Vec<HydrantWithRoute>`.
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn keine Kandidaten übergeben wurden, das 5-Sekunden-Timeout
/// mit weniger als 3 Ergebnissen abläuft, ein Worker-Task abbricht oder keine Route ermittelt werden konnte.
///
/// # Panics
///
/// Panikt, wenn der interne Ergebnis-`Mutex` durch einen abgestürzten Task vergiftet (poisoned) wurde.
pub async fn get_closest_by_route(
    user_location: &Location,
    candidate_hydrants: &[Hydrant],
    client: &Client,
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

            all_ranked_hydrants_arc
                .lock()
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
    })
        .await
    {
        Err(e) => {
            pb.println(format!(
                "[!] Timeout nach {}s. Routenberechnung nicht komplett.",
                timeout_duration.as_secs()
            ));
            let num_results = all_ranked_hydrants_arc
                .lock()
                .expect("Mutex der Ranglisten-Hydranten ist vergiftet beim Abrufen der Anzahl (len).")
                .len();

            if num_results < 3 {
                pb.finish();
                return Err(anyhow::anyhow!(
                    "Route-Berechnung konnte nicht rechtzeitig mindestens 3 Hydranten liefern (nur {})",
                    num_results
                )
                    .context(e));
            }
        }
        Ok(Err(e)) => {
            pb.finish();
            return Err(e);
        }
        Ok(Ok(_)) => {}
    };

    pb.finish();

    let mut sorted_hydrants = all_ranked_hydrants_arc
        .lock()
        .expect("Mutex der Ranglisten-Hydranten ist vergiftet beim Entleeren (drain) der Ergebnisse.")
        .drain(..)
        .collect::<Vec<HydrantWithRoute>>();

    if sorted_hydrants.is_empty() {
        anyhow::bail!("[X] Keine Routen zu Hydranten gefunden, auch nicht per Fallback.");
    }

    sorted_hydrants.sort_by(|a, b| {
        a.route_details
            .distance_meters
            .partial_cmp(&b.route_details.distance_meters)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(sorted_hydrants)
}

// --- Hilfsfunktionen (Retry) ---

/// Führt eine asynchrone Operation bei transienten Fehlern bis zu `max_retries` Mal aus.
///
/// Nutzt Exponential Backoff mit zufälligem Jitter (`0..500 ms`) zwischen den Wiederholungen.
///
/// # Arguments
///
/// * `f` - Closure, die das auszuführende Future erzeugt.
/// * `max_retries` - Maximale Gesamtanzahl an Versuchen.
/// * `base_delay` - Basis-Wartezeit für den Exponential Backoff.
///
/// # Returns
///
/// Das erfolgreiche Ergebnis `T` der ausgeführten Operation.
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn ein permanenter Fehler auftritt oder `max_retries` erreicht wurde.
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

/// Prüft, ob ein [`anyhow::Error`] einen transienten HTTP- oder Netzwerkfehler umschließt.
///
/// Als transient gelten HTTP-5xx-Fehler, HTTP 429 (`Too Many Requests`), HTTP 408 (`Request Timeout`)
/// sowie Verbindungs- und Timeout-Fehler.
///
/// # Arguments
///
/// * `e` - Der zu prüfende Fehler.
///
/// # Returns
///
/// `true`, wenn ein erneuter Versuch sinnvoll ist, andernfalls `false`.
fn is_transient_error(e: &anyhow::Error) -> bool {
    if let Some(reqwest_error) = e.downcast_ref::<reqwest::Error>() {
        if let Some(status) = reqwest_error.status() {
            if status == reqwest::StatusCode::GATEWAY_TIMEOUT
                || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
                || status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status == reqwest::StatusCode::INTERNAL_SERVER_ERROR
                || status == reqwest::StatusCode::REQUEST_TIMEOUT
                || status.is_server_error()
            {
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