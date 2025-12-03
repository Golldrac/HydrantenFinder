use std::cmp::min;
use std::env;
use std::f64::consts::PI;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use geo::{Distance, Haversine, Point};
use geo_types::Coord;
use indicatif::{ProgressBar, ProgressStyle};
use polyline;
use rand::Rng;
use reqwest::{self, StatusCode};
use serde_json;
use tokio::task;
use tokio::time::sleep;

use crate::models::{
    Hydrant, Location, LocationResponse, MapLine, MapMarker, MapRequest, ORSResponse, OSRMResponse,
    RankedHydrant, RouteDetails,
};

/// Der maximale Suchradius in Metern für Hydranten.
const MAX_RADIUS: u32 = 1_500;
/// Die Genauigkeit, mit der Polylines kodiert werden sollen.
const POLYLINE_PRECISION: u32 = 6;
/// Der maximale Abstand in Metern, bei dem ein Hydrant als "erreicht" gilt und keine Routenberechnung nötig ist.
const PROXIMITY_THRESHOLD_METERS: f64 = 5.0;
/// Die Breite der generierten statischen Karte in Pixeln (für Zoom-Berechnung).
const MAP_WIDTH_PIXELS: f64 = 800.0;
/// Der maximale Zoomlevel für die Stadia Maps API.
const MAX_ZOOM: u32 = 18;

// --- Hydranten-Suche (Overpass) ---

/// Ruft Hydranten im angegebenen Radius um die übergebene Position von der Overpass API ab.
///
/// Führt interne Retries bei transienten Fehlern durch. Verdoppelt den Suchradius (`initial_radius_m`)
/// bei jedem erfolglosen Versuch, bis mindestens 5 Hydranten gefunden oder der `MAX_RADIUS` erreicht wird.
///
/// # Arguments
///
/// * `location` - Die Startposition.
/// * `initial_radius_m` - Der anfängliche Suchradius in Metern.
///
/// # Returns
///
/// Eine `Vec<Hydrant>` der gefundenen Hydranten.
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn die Overpass API nach allen Retries einen nicht erfolgreichen Statuscode zurückgibt.
pub async fn find_hydrants(location: &Location, initial_radius_m: u32) -> Result<Vec<Hydrant>> {
    // ... (Funktionsimplementierung unverändert)
    let lat = location.latitude;
    let lon = location.longitude;
    let mut radius_m = initial_radius_m;
    let mut attempt = 1;
    let base_delay = Duration::from_millis(1300);

    loop {
        let query = format!(
            r#"[out:json][timeout:60];
            (
            node(around:{}, {}, {})[emergency=fire_hydrant];
            );
            out tags center;"#,
            radius_m, lat, lon
        );

        let url = "https://overpass-api.de/api/interpreter";

        let client = reqwest::Client::new();

        let response = retry_on_failure(
            || {
                // Die Closure MUSS die benötigten Variablen klonen/übernehmen,
                // da sie mehrmals in unterschiedlichen async-Tasks ausgeführt werden könnte.
                let client_clone = client.clone();
                let url_clone = url;
                let query_clone = query.clone();

                // Dies ist die asynchrone Funktion, die bei jedem Versuch neu ausgeführt wird.
                async move {
                    // Die Antwort des Clients muss in ein anyhow::Result<Response> konvertiert werden
                    // (reqwest::Error -> anyhow::Error).
                    client_clone
                        .post(url_clone)
                        .body(query_clone)
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
                // Der Fehler kommt von retry_on_failure
                anyhow::bail!("❌ Overpass-API-Aufruf fehlgeschlagen: {}", e);
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .context("Overpass: Kein Antwort-Body verfügbar")?;
            anyhow::bail!(
                "❌ Overpass-API-Fehler (Status: {}). Antwort: {}",
                status,
                body
            );
        }

        let response_body = response.json::<LocationResponse>().await?;

        let hydrants = response_body.elements;

        if hydrants.len() >= 5 || radius_m >= MAX_RADIUS {

            return Ok(hydrants);

        }

        let jitter = Duration::from_millis(rand::rng().random_range(0..500));
        let delay = base_delay * (2_i32.pow(attempt) as u32) + jitter;
        radius_m = min(initial_radius_m * (2_i32.pow(attempt) as u32), MAX_RADIUS);
        sleep(delay).await;
        attempt += 1;

    }
}

/// Ruft die nächstgelegenen Hydranten (Luftlinie) ab.
///
/// Die Hydranten werden zuerst über `find_hydrants` gesucht (aktuell mit festem Radius von 500m),
/// dann nach Haversine-Distanz sortiert und auf die gewünschte Anzahl begrenzt.
///
/// # Arguments
///
/// * `location` - Die Startposition des Benutzers.
/// * `amount` - Die maximale Anzahl der zurückzugebenden Hydranten (Standard: 5).
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
) -> Result<Vec<Hydrant>> {
    // ... (Funktionsimplementierung unverändert)
    let max_len = amount.unwrap_or(5);
    // Reduzierter Radius auf 500m, da die Routenberechnung parallel erfolgt und sonst zu lange dauern könnte
    let radius_m = 500;

    let hydrant_locations = find_hydrants(location, radius_m)
        .await
        .context("Fehler beim Abruf der Hydranten")?;

    if hydrant_locations.is_empty() {
        // Einheitliche Fehlermeldung
        anyhow::bail!(
            "🔍 Keine Hydranten in der Nähe gefunden (Radius: {}m).",
            radius_m
        );
    }

    let user_location_point = Point::new(location.longitude, location.latitude);

    let mut nearest_hydrants: Vec<(Hydrant, f64)> = hydrant_locations
        .into_iter()
        .map(|hydrant| {
            let hydrant_point = Point::new(hydrant.longitude, hydrant.latitude);
            // Haversine-Distanz ist im geo-Crate definiert und wird direkt verwendet.
            let distance = Haversine.distance(user_location_point, hydrant_point);
            (hydrant, distance)
        })
        .collect();

    nearest_hydrants.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

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
    client: &reqwest::Client,
    start: &Location,
    destination: &Hydrant,
) -> Result<RouteDetails> {
    // ... (Funktionsimplementierung unverändert)
    let url = format!(
        "https://router.project-osrm.org/route/v1/foot/{},{};{},{}?steps=false&snapping=any&overview=full",
        start.longitude, start.latitude, destination.longitude, destination.latitude
    );

    let response = retry_on_failure(
        || {
            // Die Closure MUSS die benötigten Variablen klonen/übernehmen,
            // da sie mehrmals in unterschiedlichen async-Tasks ausgeführt werden könnte.
            let client_clone = client.clone();
            let url_clone = url.clone();

            // Dies ist die asynchrone Funktion, die bei jedem Versuch neu ausgeführt wird.
            async move {
                // Die Antwort des Clients muss in ein anyhow::Result<Response> konvertiert werden
                // (reqwest::Error -> anyhow::Error).
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

    // Behandelt gescheiterte Retries (Netzwerkfehler)
    let response = match response {
        Ok(r) => r,
        Err(e) => {
            // Einheitliche Log-Ausgabe
            eprintln!("⚠️ OSRM-Netzwerkfehler (nach Retries): {}. Versuche ORS...", e);
            // Fallback auf ORS
            return fetch_route_ors(client, start, destination).await;
        }
    };

    // Behandelt HTTP Status Code Fehler
    if !response.status().is_success() {
        eprintln!(
            "⚠️ OSRM-API-Fehler (Status: {}). Versuche ORS...",
            response.status()
        );
        // Fallback auf ORS
        return fetch_route_ors(client, start, destination).await;
    }

    let response_body = response.json::<OSRMResponse>().await?;

    // Prüft, ob eine gültige Route vorhanden ist, ansonsten Fallback
    if let Some(route) = response_body.routes.into_iter().next() {
        if let Some(leg) = route.legs.into_iter().next() {
            if leg.distance == 0.0 {
                // 0m-Route Fallback
                eprintln!("⚠️ OSRM lieferte 0m-Route. Versuche ORS...");
                return fetch_route_ors(client, start, destination).await;
            }
            return Ok(RouteDetails {
                distance_meters: leg.distance,
                geometry_polyline: route.geometry,
            });
        }
    }

    // Fallback, wenn keine Route im Body gefunden wurde
    eprintln!("⚠️ OSRM lieferte keine Route im Body. Versuche ORS...");
    fetch_route_ors(client, start, destination).await
}

/// Berechnet die Route zwischen Start und Ziel mithilfe der OpenRouteService (ORS) API.
///
/// ORS dient als Fallback für OSRM und erfordert einen API-Schlüssel (`ORS_API_KEY`).
/// Führt interne Retries bei transienten Fehlern durch. Meldet ORS eine 0m-Route,
/// wird die Haversine-Distanz (Luftlinie) als Fallback zurückgegeben (falls > 10m).
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
    client: &reqwest::Client,
    start: &Location,
    destination: &Hydrant,
) -> Result<RouteDetails> {
    // ... (Funktionsimplementierung unverändert)
    let api_key =
        env::var("ORS_API_KEY").context("❌ ORS_API_KEY Umgebungsvariable muss gesetzt sein. Routing nicht möglich.")?;

    let url = format!(
        "https://api.openrouteservice.org/v2/directions/foot-walking?api_key={}&start={},{}&end={},{}&snapping=any",
        api_key, start.longitude, start.latitude, destination.longitude, destination.latitude
    );

    let response = retry_on_failure(
        || {
            // Die Closure MUSS die benötigten Variablen klonen/übernehmen,
            // da sie mehrmals in unterschiedlichen async-Tasks ausgeführt werden könnte.
            let client_clone = client.clone();
            let url_clone = url.clone();

            // Dies ist die asynchrone Funktion, die bei jedem Versuch neu ausgeführt wird.
            async move {
                // Die Antwort des Clients muss in ein anyhow::Result<Response> konvertiert werden
                // (reqwest::Error -> anyhow::Error).
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
        .await; // Das Ergebnis der `retry_on_failure` Funktion muss geawaited werden!

    // Nun behandeln wir das finale Ergebnis:
    let response = match response {
        Ok(r) => r,
        Err(e) => {
            // Einheitliche Fehlermeldung
            anyhow::bail!("❌ ORS-API-Aufruf fehlgeschlagen: {}", e);
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .context("ORS: Kein Antwort-Body verfügbar")?;
        // Einheitliche Fehlermeldung
        anyhow::bail!("❌ ORS-API-Fehler (Status: {}). Antwort: {}", status, body);
    }

    let body_text = response.text().await?;

    let response_body: ORSResponse = match serde_json::from_str(&body_text) {
        Ok(body) => body,
        Err(e) => {
            anyhow::bail!(
                // Einheitliche Fehlermeldung
                "❌ ORS-JSON-Decode-Fehler: {}. Ungültige JSON-Antwort (Ausschnitt): {}",
                e,
                &body_text[..body_text.len().min(100)] // Ausschnitt für sauberere Fehlerausgabe
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

            if haversine_dist > 10.0 {
                eprintln!(
                    "📐 Fallback: ORS-Route war 0m. Gebe Luftlinie ({:.2}m) zurück.",
                    haversine_dist
                );
                return Ok(RouteDetails {
                    distance_meters: haversine_dist,
                    geometry_polyline: String::new(), // Keine Polyline für Luftlinie bei Fallback
                });
            }
        }

        return Ok(RouteDetails {
            distance_meters: route_distance,
            geometry_polyline,
        });
    }

    // Einheitliche Fehlermeldung
    anyhow::bail!("❌ Keine Route (Feature) in der ORS-Antwort gefunden.");
}

// --- Ranking / Routen-Aggregator ---

/// Ermittelt den am besten bewerteten Hydranten unter den Kandidaten.
///
/// Die Bewertung erfolgt basierend auf der berechneten Geh-Route (unter Verwendung von `fetch_route_osrm` mit ORS-Fallback)
/// und den Hydranten-Eigenschaften (`calculate_score`).
///
/// Parallelisierung: Die Routenberechnung für jeden Kandidaten erfolgt asynchron und parallel
/// mithilfe von `tokio::task::spawn`.
///
/// Optimierung: Wenn der Hydrant, der per Luftlinie am nächsten liegt,
/// bereits innerhalb des `PROXIMITY_THRESHOLD_METERS` (5m) liegt, wird sofort diese Position
/// ohne Routenberechnung als gültiger, gerankter Hydrant hinzugefügt.
///
/// # Arguments
///
/// * `user_location` - Die Startposition des Benutzers.
/// * `candidate_hydrants` - Eine Liste von Hydranten, die in Betracht gezogen werden sollen (typischerweise die nach Luftlinie nächsten).
///
/// # Returns
///
/// Eine sortierte `Vec<RankedHydrant>` mit dem besten (niedrigsten Score) an erster Stelle.
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn keine gültige Route zu einem der Hydranten gefunden werden konnte.
pub async fn get_closest_by_route(
    user_location: &Location,
    candidate_hydrants: &[Hydrant],
) -> Result<Vec<RankedHydrant>> {
    // ... (Funktionsimplementierung unverändert)
    if candidate_hydrants.is_empty() {
        anyhow::bail!("❌ Keine Hydranten-Kandidaten zur Routenberechnung vorhanden.");
    }

    // Benutzerposition als Punkt für die Luftlinienprüfung
    let user_location_point = Point::new(user_location.longitude, user_location.latitude);

    let spinner_style = ProgressStyle::with_template("{spinner:.cyan.bold} {msg}")?;

    // Erstellen Sie den Spinner zur Visualisierung der parallelen Abarbeitung
    let pb = ProgressBar::new(candidate_hydrants.len() as u64); // Progress Bar mit Gesamtzahl der Aufgaben
    pb.set_style(spinner_style);
    pb.set_message("🧭 Berechne Routen zu Hydranten...");
    pb.enable_steady_tick(Duration::from_millis(100));

    let client = reqwest::Client::builder()
        .user_agent("HydrantenFinder/1.0")
        .build()?;

    let all_ranked_hydrants_arc: Arc<Mutex<Vec<RankedHydrant>>> = Arc::new(Mutex::new(Vec::new()));

    // Sammeln Sie alle Handles, um später auf den Abschluss aller Tasks zu warten
    let mut join_handles = Vec::new();

    // Die Benutzerposition muss geklont werden, da jede Task den *Besitz* an ihren Daten benötigt
    let user_location_owned = user_location.clone();

    for hydrant in candidate_hydrants {
        // Klon der ProgressBar, um den Fortschritt von der Task aus zu aktualisieren
        let pb_clone = pb.clone();

        let hydrant_point = Point::new(hydrant.longitude, hydrant.latitude);

        let haversine_dist = Haversine.distance(user_location_point, hydrant_point);

        // Optimierung: Wenn Hydrant in Reichweite ist, sofort als Ergebnis hinzufügen
        if haversine_dist <= PROXIMITY_THRESHOLD_METERS {
            let candidate = RankedHydrant::new(
                hydrant.clone(),
                RouteDetails {
                    distance_meters: haversine_dist,
                    geometry_polyline: String::new(),
                },
            );

            all_ranked_hydrants_arc.lock().unwrap().push(candidate);
            pb_clone.inc(1);
            continue;
        }

        // --- Klonen der benötigten Variablen für die asynchrone Task ---
        let client_clone = client.clone();
        let hydrant_clone = hydrant.clone();
        let user_location_task = user_location_owned.clone();
        let all_ranked_hydrants_clone = Arc::clone(&all_ranked_hydrants_arc);

        // task::spawn startet die Routenberechnung asynchron im Tokio Worker Pool
        let handle = task::spawn(async move {
            match fetch_route_osrm(&client_clone, &user_location_task, &hydrant_clone).await {
                Ok(details) => {
                    let candidate = RankedHydrant::new(hydrant_clone, details);
                    all_ranked_hydrants_clone.lock().unwrap().push(candidate);
                }
                Err(e) => {
                    // Fehler loggen, aber nicht die gesamte Task abbrechen
                    pb_clone.println(format!("⚠️ Route zu Hydrant übersprungen: {}", e));
                }
            }
            pb_clone.inc(1); // Fortschrittszähler erhöhen
        });

        join_handles.push(handle);
    }

    // Warten Sie asynchron auf den Abschluss aller Tasks
    for handle in join_handles {
        // `await?` prüft auf Paniken in der Task
        handle.await?;
    }

    // Die Progress Bar beenden
    pb.finish();

    // Die gesammelten Ergebnisse aus dem Mutex extrahieren
    let mut sorted_hydrants = all_ranked_hydrants_arc.lock().unwrap().drain(..).collect::<Vec<RankedHydrant>>();

    if sorted_hydrants.is_empty() {
        anyhow::bail!("❌ Keine Routen zu Hydranten gefunden, auch nicht per Fallback.");
    }

    // Sortieren nach dem Score (der kleinere Score ist besser)
    sorted_hydrants.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal));

    Ok(sorted_hydrants)
}

// --- Karten-Generierung (Stadia Maps) ---

/// Ruft eine statische Karte von der Stadia Maps API ab, die die Route anzeigt.
///
/// Führt interne Retries bei transienten Fehlern durch. Die Route wird im
/// Request-Body als JSON übermittelt.
///
/// # Arguments
///
/// * `user` - Die Position des Benutzers (roter Marker).
/// * `hydrant` - Der beste gerankte Hydrant (blauer Marker mit 'H').
/// * `width` - Die gewünschte Breite der Karte in Pixeln.
/// * `height` - Die gewünschte Höhe der Karte in Pixeln.
///
/// # Returns
///
/// Die Bilddaten der Karte als `Vec<u8>` (PNG-Format).
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn die `STADIAMAPS_API_KEY` Umgebungsvariable fehlt oder
/// die API nach allen Retries einen Fehlerstatus zurückgibt.
pub async fn get_map(
    user: &Location,
    hydrant: &RankedHydrant,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    // ... (Funktionsimplementierung unverändert)
    let api_key = env::var("STADIAMAPS_API_KEY")
        .context("❌ STADIAMAPS_API_KEY Umgebungsvariable muss gesetzt sein. Kartenerstellung nicht möglich.")?;

    let hydrant_loc = &hydrant.hydrant;
    let polyline_param = &hydrant.route_details.geometry_polyline;
    // Zoom-Level berechnen, damit beide Punkte sichtbar sind
    let zoom = calculate_zoom_level(user, hydrant_loc);

    // Berechnung des Kartenzentrums
    let center_lat = (user.latitude + hydrant_loc.latitude) / 2f64;
    let center_lon = (user.longitude + hydrant_loc.longitude) / 2f64;

    let request_body = MapRequest {
        center: format!("{},{}", center_lat, center_lon),
        // Die Karte wird in doppelter Auflösung angefordert (@2x)
        size: format!("{}x{}@2x", width, height),
        zoom,
        markers: vec![
            // Benutzer-Marker (rot)
            MapMarker {
                lat: user.latitude,
                lon: user.longitude,
                color: Some("red".to_string()),
                label: None,
            },
            // Hydrant-Marker (blau mit 'H')
            MapMarker {
                lat: hydrant_loc.latitude,
                lon: hydrant_loc.longitude,
                color: Some("blue".to_string()),
                label: Some("H".to_string()),
            },
        ],
        // Füge die Routen-Polyline nur hinzu, wenn sie existiert (nicht bei Haversine-Fallback)
        lines: if polyline_param.is_empty() {
            vec![]
        } else {
            vec![MapLine {
                shape: polyline_param.clone(),
                stroke_color: Some("red".to_string()),
                stroke_width: Some(5),
            }]
        },
    };

    let url = format!(
        "https://tiles.stadiamaps.com/static_cacheable/alidade_smooth.png?api_key={}",
        api_key
    );

    let client = reqwest::Client::new();

    let response = retry_on_failure(
        || {
            // Die Closure MUSS die benötigten Variablen klonen/übernehmen,
            // da sie mehrmals in unterschiedlichen async-Tasks ausgeführt werden könnte.
            let client_clone = client.clone();
            let url_clone = url.clone();
            let request_body_clone = request_body.clone();

            // Dies ist die asynchrone Funktion, die bei jedem Versuch neu ausgeführt wird.
            async move {
                // Die Antwort des Clients muss in ein anyhow::Result<Response> konvertiert werden
                // (reqwest::Error -> anyhow::Error).
                client_clone
                    .post(&url_clone)
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .json(&request_body_clone)
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
            // Einheitliche Fehlermeldung
            anyhow::bail!("❌ Stadia-Maps-API-Aufruf fehlgeschlagen: {}", e);
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        // Versuch, den Fehler-Body abzurufen. Ungültige/Fehlende Bodies werden ignoriert,
        // um keinen doppelten Fehler beim Logging zu erzeugen.
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "Kein Antwort-Body verfügbar".to_string());

        // Einheitliche Fehlermeldung
        anyhow::bail!(
            "❌ Stadia-Maps-API-Fehler (Status: {}). Antwort: {}",
            status,
            body
        );
    }

    // Nur, wenn der Status erfolgreich (2xx) ist, die Bilddaten abrufen.
    Ok(response.bytes().await?.to_vec())
}

/// Berechnet einen geeigneten Zoomlevel für die statische Karte,
/// sodass beide Standorte auf die Karte passen.
///
/// Nutzt eine Näherungsformel, die auf der horizontalen Distanz zwischen den Punkten basiert.
///
/// # Arguments
///
/// * `loc1` - Die erste Position (z.B. Benutzer).
/// * `loc2` - Die zweite Position (z.B. Hydrant).
///
/// # Returns
///
/// Der berechnete Zoomlevel als `u32`, begrenzt auf $0$ bis $18$.
pub fn calculate_zoom_level(loc1: &Location, loc2: &Hydrant) -> u32 {
    // ... (Funktionsimplementierung unverändert)
    let mid_lat_rad = (loc1.latitude.to_radians() + loc2.latitude.to_radians()) / 2.0;
    let max_lon_dist = (loc1.longitude - loc2.longitude).abs();

    // Näherungsweise Berechnung der Meter pro Längengrad auf der mittleren Breite (basierend auf Erdradius)
    let meters_per_lon_degree = 6371000.0 * 2.0 * PI * mid_lat_rad.cos() / 360.0;
    let horizontal_distance_m = max_lon_dist * meters_per_lon_degree;

    // Vermeide Logarithmus von Null bei gleicher Position
    if horizontal_distance_m == 0.0 {
        return MAX_ZOOM;
    }

    // Formel zur Abschätzung des Zoomlevels basierend auf der Distanz und der Kartenbreite
    let zoom_factor = 450.0; // Faktor, der die Auflösung der Kachelgröße berücksichtigt
    let zoom_f64 = MAP_WIDTH_PIXELS.log2() + (zoom_factor / horizontal_distance_m).log2();

    let final_zoom = zoom_f64.floor() as u32;

    final_zoom.clamp(0, MAX_ZOOM)
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
    // Die aktuelle Anzahl der Versuche
    let mut attempt = 1;

    loop {
        // 1. Funktion ausführen
        match f().await {
            // 2. Erfolg: Ergebnis sofort zurückgeben
            Ok(result) => return Ok(result),

            // 3. Fehler: Fehlerbehandlung
            Err(e) => {
                // 1. Ist der Fehler NICHT transient? -> Schleife sofort abbrechen und Fehler zurückgeben.
                if !is_transient_error(&e) {
                    // Wichtig: Wir returnen den Fehler, damit der Aufrufer ihn matchen kann.
                    return Err(e).context("API-Aufruf fehlgeschlagen: Permanenter Fehler.");
                }

                // 2. Ist es ein transienter Fehler UND der letzte Versuch? -> Abbrechen nach Retries.
                if attempt >= max_retries {
                    // Letzter Versuch fehlgeschlagen: Fehler ausgeben und beenden
                    anyhow::bail!(
                        "❌ API-Aufruf nach {} Versuchen fehlgeschlagen. Letzter Fehler: {}",
                        max_retries,
                        e
                    );
                }
                let jitter = Duration::from_millis(rand::rng().random_range(0..500));
                let delay = base_delay * (2_i32.pow(attempt) as u32) + jitter;
                // 3. Transienter Fehler und weitere Versuche übrig -> Warten und fortsetzen.
                println!(
                    "⚠️ Versuch {}/{}: API fehlgeschlagen. Warte {:.1}s...",
                    attempt,
                    max_retries,
                    delay.as_secs_f32()
                );
                sleep(delay).await;

                // Zähler erhöhen und die `loop` startet automatisch den nächsten Versuch
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
    // Versuche, den Fehler in einen reqwest::Error umzuwandeln
    if let Some(reqwest_error) = e.downcast_ref::<reqwest::Error>() {
        // Prüfe auf HTTP Status Code Fehler
        if let Some(status) = reqwest_error.status() {
            // Einheitliche Fehler-Prüfung
            return status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS;
        }

        // Prüfe auf Netzwerkfehler/Timeouts
        return reqwest_error.is_timeout() || reqwest_error.is_connect();
    }

    // Wenn es kein reqwest-Fehler ist oder der Typ unbekannt ist, behandeln wir ihn als fatal
    false
}