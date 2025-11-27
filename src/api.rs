use std::f64::consts::PI;
use std::env;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use geo::{Haversine, HaversineDistance, Point};
use geo_types::Coord;
use indicatif::{ProgressBar, ProgressStyle};
use polyline;
use reqwest;
use serde_json;
use std::sync::{Arc, Mutex};
use reqwest::StatusCode;
use tokio::task;
use tokio::time::sleep;

use crate::models::{HydrantTags, Location, LocationResponse, MapLine, MapMarker, MapRequest, ORSResponse, OSRMResponse, RankedHydrant, RouteDetails};

/// Die Genauigkeit, mit der Polylines kodiert werden sollen.
const POLYLINE_PRECISION: u32 = 6;
/// Der maximale Abstand in Metern, bei dem ein Hydrant als "erreicht" gilt und keine Routenberechnung nötig ist.
const PROXIMITY_THRESHOLD_METERS: f64 = 5.0;
/// Die Breite der generierten statischen Karte in Pixeln (für Zoom-Berechnung).
const MAP_WIDTH_PIXELS: f64 = 800.0;
/// Der maximale Zoomlevel für die Stadia Maps API.
const MAX_ZOOM: u32 = 18;


/// Ruft Hydranten im angegebenen Radius um die übergebene Position von der Overpass API ab.
///
/// # Arguments
///
/// * `location` - Die Startposition.
/// * `radius_m` - Der Suchradius in Metern.
///
/// # Returns
///
/// Eine `Vec<Location>` der gefundenen Hydranten.
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn die Overpass API einen nicht erfolgreichen Statuscode zurückgibt.
pub async fn find_hydrants(location: &Location, radius_m: u32) -> Result<Vec<Location>> {
    let lat = location.latitude;
    let lon = location.longitude;
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
                client_clone.post(url_clone).body(query_clone).send().await.map_err(anyhow::Error::from)
            }
        },
        2, // Max. 2 Versuche (1. Versuch + 1 Retry)
        Duration::from_secs(2),
    )
        .await;

    let response = match response {
        Ok(r) => r,
        Err(e) => {

            // Dein Fallback von OSRM auf ORS bleibt hier erhalten!
            anyhow::bail!("❌ Overpass API Aufruf fehlgeschlagen nach Retries: {}", e);
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .context("Overpass: Kein Antwort-Body verfügbar")?;
        anyhow::bail!(
            "❌ Overpass API Fehler (Status: {}). Antwort: {}",
            status,
            body
        );
    }

   let response_body = response.json::<LocationResponse>().await?;

    Ok(response_body.elements)
}

/// Ruft die nächstgelegenen Hydranten (Luftlinie) ab.
///
/// Die Hydranten werden zuerst über `find_hydrants` in einem festen Radius gesucht,
/// dann nach Haversine-Distanz sortiert und auf die gewünschte Anzahl begrenzt.
///
/// # Arguments
///
/// * `location` - Die Startposition des Benutzers.
/// * `amount` - Die maximale Anzahl der zurückzugebenden Hydranten (Standard: 5).
///
/// # Returns
///
/// Eine `Vec<Location>` der nächstgelegenen Hydranten.
pub async fn get_nearest_hydrants(location: &Location, amount: Option<usize>) -> Result<Vec<Location>> {
    let max_len = amount.unwrap_or(5);
    let radius_m = 500;

    let hydrant_locations = find_hydrants(location, radius_m)
        .await
        .context("Fehler beim Abruf der Hydranten")?;

    if hydrant_locations.is_empty() {
        anyhow::bail!("🔍 Keine Hydranten in der Nähe gefunden (Radius: {}m).", radius_m);
    }


    let user_location_point = Point::new(location.longitude, location.latitude);

    let mut nearest_hydrants: Vec<(Location, f64)> = hydrant_locations
        .into_iter()
        .map(|hydrant| {
            let hydrant_point = Point::new(hydrant.longitude, hydrant.latitude);
            // HaversineDistance ist der korrekte Trait für diese Methode
            let distance = user_location_point.haversine_distance(&hydrant_point);
            (hydrant, distance)
        })
        .collect();

    nearest_hydrants.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let final_coords: Vec<Location> = nearest_hydrants
        .into_iter()
        .take(max_len)
        .map(|(coord, _distance)| coord)
        .collect();

    Ok(final_coords)
}

/// Berechnet die Route zwischen Start und Ziel mithilfe der OSRM (Open Source Routing Machine) API.
///
/// **Wichtig:** Bei Netzwerkfehlern, HTTP-Fehler-Statuscodes oder ungültiger/0m Route
/// wird automatisch auf `fetch_route_ors` zurückgefallen.
///
/// # Arguments
///
/// * `client` - Ein geteilter reqwest Client.
/// * `start` - Die Startposition.
/// * `destination` - Die Zielposition.
///
/// # Returns
///
/// Die `RouteDetails` der berechneten Route.
pub async fn fetch_route_osrm(client: &reqwest::Client, start: &Location, destination: &Location) -> Result<RouteDetails> {
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
                client_clone.get(&url_clone).send().await.map_err(anyhow::Error::from)
            }
        },
        2, // Max. 2 Versuche (1. Versuch + 1 Retry)
        Duration::from_secs(2),
    )
        .await;

    // Das match hier fängt jetzt ALLES ab (permanente Fehler oder gescheiterte Retries)
    let response = match response {
        Ok(r) => r,
        Err(e) => {
            // Dieser Pfad wird jetzt bei ALLEN Fehlern erreicht,
            // die entweder nicht transient sind ODER nach max_retries aufgetreten sind.
            eprintln!("🌐 OSRM-Fehler: {}. Versuche ORS...", e);
            // -> alter Fehlerfall (Fallback auf ORS)
            return fetch_route_ors(client, start, destination).await;
        }
    };

    // ALTER FEHLERFALL: Statuscode-Prüfung (z.B. OSRM liefert 200, aber leere Route)
    // Wenn die Antwort ok war, prüfen wir den Statuscode der Antwort (alte Logik)
    if !response.status().is_success() {
        eprintln!("🚦 OSRM-Statusfehler: {}. Versuche ORS...", response.status());
        // -> alter Fehlerfall (Fallback auf ORS)
        return fetch_route_ors(client, start, destination).await;
    }

    let response_body = response.json::<OSRMResponse>().await?;

    // Prüft, ob eine gültige Route vorhanden ist, ansonsten Fallback
    if let Some(route) = response_body.routes.into_iter().next() {
        if let Some(leg) = route.legs.into_iter().next() {
            if leg.distance == 0.0 {
                return fetch_route_ors(client, start, destination).await;
            }
            return Ok(RouteDetails {
                distance_meters: leg.distance,
                geometry_polyline: route.geometry,
            });
        }
    }

    // Fallback, wenn keine Route im Body gefunden wurde
    fetch_route_ors(client, start, destination).await
}

/// Berechnet die Route zwischen Start und Ziel mithilfe der OpenRouteService (ORS) API.
///
/// ORS dient als Fallback für OSRM und erfordert einen API-Schlüssel (`ORS_API_KEY`).
/// Meldet ORS eine 0m-Route, wird die Haversine-Distanz (Luftlinie) zurückgegeben,
/// falls diese Luftlinie größer als 10m ist. Andernfalls wird der Fehler ausgegeben.
///
/// # Arguments
///
/// * `client` - Ein geteilter reqwest Client.
/// * `start` - Die Startposition.
/// * `destination` - Die Zielposition.
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn die `ORS_API_KEY` Umgebungsvariable fehlt oder
/// die API einen Fehlerstatus zurückgibt.
pub async fn fetch_route_ors(client: &reqwest::Client, start: &Location, destination: &Location) -> Result<RouteDetails> {
    let api_key =
        env::var("ORS_API_KEY").context("❌ ORS_API_KEY Umgebungsvariable muss gesetzt sein. Routing nicht möglich.")?;

    let url = format!(
        "https://api.openrouteservice.org/v2/directions/foot-walking?api_key={}&start={},{}&end={},{}&snapping=any",
        api_key, start.longitude, start.latitude, destination.longitude, destination.latitude
    );

    let response = client.get(&url).send().await?;let response = retry_on_failure(
        || {
            // Die Closure MUSS die benötigten Variablen klonen/übernehmen,
            // da sie mehrmals in unterschiedlichen async-Tasks ausgeführt werden könnte.
            let client_clone = client.clone();
            let url_clone = url.clone();

            // Dies ist die asynchrone Funktion, die bei jedem Versuch neu ausgeführt wird.
            async move {
                // Die Antwort des Clients muss in ein anyhow::Result<Response> konvertiert werden
                // (reqwest::Error -> anyhow::Error).
                client_clone.get(&url_clone).send().await.map_err(anyhow::Error::from)
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
            anyhow::bail!("❌ ORS-Netzwerk-/API-Fehler (nach Retries): {}", e);
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .context("ORS: Kein Antwort-Body verfügbar")?;
        anyhow::bail!("❌ ORS API Fehler (Status: {}). Antwort: {}", status, body);
    }

    let body_text = response.text().await?;

    let response_body: ORSResponse = match serde_json::from_str(&body_text) {
        Ok(body) => body,
        Err(e) => {
            anyhow::bail!(
                "ORS Decode Fehler: {}. Ungültige JSON-Antwort (Ausschnitt): {}",
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
            let haversine_dist = start_point.haversine_distance(&dest_point);

            if haversine_dist > 10.0 {
                eprintln!(
                    "📐 Fallback: ORS-Route war 0m. Rückgabe Luftlinie ({:.2}m).",
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

    anyhow::bail!("🗺️ Keine Route (Feature) in der ORS-Antwort gefunden.");
}

/// Ermittelt den nächstgelegenen Hydranten unter den Kandidaten,
/// basierend auf der berechneten Geh-Route (unter Verwendung von `fetch_route_osrm` mit ORS-Fallback).
///
/// Parallelisierung: Die Routenberechnung für jeden Kandidaten erfolgt asynchron und parallel
/// mithilfe von `tokio::task::spawn`. Der beste Hydrant wird durch `Arc<Mutex<T>>` sicher über Tasks hinweg aktualisiert.
///
/// Optimierung: Wenn der Hydrant, der per Luftlinie am nächsten liegt,
/// bereits innerhalb des `PROXIMITY_THRESHOLD_METERS` (5m) liegt, wird sofort diese Position
/// ohne Routenberechnung zurückgegeben.
///
/// # Arguments
///
/// * `user_location` - Die Startposition des Benutzers.
/// * `candidate_hydrants` - Eine Liste von Hydranten, die in Betracht gezogen werden sollen (typischerweise die nach Luftlinie nächsten).
///
/// # Returns
///
/// Die `RankedHydrant` mit der kürzesten Geh-Distanz.
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn keine gültige Route zu einem der Hydranten gefunden werden konnte.
pub async fn get_closest_by_route(user_location: &Location, candidate_hydrants: &[Location]) -> Result<RankedHydrant> {
    if candidate_hydrants.is_empty() {
        anyhow::bail!("🗺️ Keine Hydranten-Kandidaten zur Routenberechnung vorhanden.");
    }

    // Prüfen, ob der nächste (Luftlinie) Hydrant bereits in Reichweite ist
    let user_location_point = Point::new(user_location.longitude, user_location.latitude);
    let first_hydrant_location = &candidate_hydrants[0];

    let hydrant_point = Point::new(
        first_hydrant_location.longitude,
        first_hydrant_location.latitude,
    );
    let haversine_dist = user_location_point.haversine_distance(&hydrant_point);

    if haversine_dist <= PROXIMITY_THRESHOLD_METERS {
        // Konsolenausgabe angepasst, um zum Stil zu passen
        println!(
            "✅ Nächster Hydrant (< {}m) in direkter Reichweite. Überspringe Routenberechnung.",
            PROXIMITY_THRESHOLD_METERS
        );
        return Ok(RankedHydrant {
            hydrant: first_hydrant_location.clone(),
            route_details: RouteDetails {
                distance_meters: haversine_dist,
                geometry_polyline: String::new(),
            },
        });
    }

    let spinner_style = ProgressStyle::with_template("{spinner:.cyan.bold} {msg}").unwrap();

    // Erstellen Sie den Spinner zur Visualisierung der parallelen Abarbeitung
    let pb = ProgressBar::new(candidate_hydrants.len() as u64); // Progress Bar mit Gesamtzahl der Aufgaben
    pb.set_style(spinner_style);
    pb.set_message(
        "🧭 Berechne Routen zu Hydranten..."
    );
    pb.enable_steady_tick(std::time::Duration::from_millis(100));

    let client = reqwest::Client::builder()
        .user_agent("HydrantenFinder/1.0")
        .build()?;

    // Bestes Ergebnis, geschützt durch Arc<Mutex> für sicheren, gleichzeitigen Zugriff
    let best_hydrant_arc: Arc<Mutex<Option<RankedHydrant>>> = Arc::new(Mutex::new(None));

    // Sammeln Sie alle Handles, um später auf den Abschluss aller Tasks zu warten
    let mut join_handles = Vec::new();

    // Die Benutzerposition muss geklont werden, da jede Task den *Besitz* an ihren Daten benötigt
    let user_location_owned = user_location.clone();

    for hydrant in candidate_hydrants {
        // --- Klonen der benötigten Variablen für die asynchrone Task ---
        // Klon des reqwest-Clients (effizient durch interne Arc-Verwendung)
        let client_clone = client.clone();

        // Klon des Mutex-Arcs, um den gemeinsamen Zugriff zu teilen
        let best_hydrant_clone = Arc::clone(&best_hydrant_arc);

        // Klon der ProgressBar, um den Fortschritt von der Task aus zu aktualisieren
        let pb_clone = pb.clone();

        // Klon des aktuellen Hydranten (Besitzübernahme)
        let hydrant_clone = hydrant.clone();

        // Klon der Benutzerposition für die Routenberechnung in dieser Task
        let user_location_task = user_location_owned.clone();

        // task::spawn startet die Routenberechnung asynchron im Tokio Worker Pool
        let handle = task::spawn(async move {

            match fetch_route_osrm(&client_clone, &user_location_task, &hydrant_clone).await {
                Ok(details) => {
                    let candidate = RankedHydrant {
                        hydrant: hydrant_clone,
                        route_details: details,
                    };

                    // Sperren des Mutex für den veränderlichen Zugriff auf den besten Hydranten
                    let mut best_hydrant_guard = best_hydrant_clone.lock().unwrap();

                    match best_hydrant_guard.as_ref() {
                        None => {
                            // Erster gültiger Hydrant gefunden
                            *best_hydrant_guard = Some(candidate);
                        }
                        Some(current_best) => {
                            // Vergleichen und gegebenenfalls aktualisieren
                            if candidate.route_details.distance_meters < current_best.route_details.distance_meters {
                                *best_hydrant_guard = Some(candidate);
                            }
                        }
                    }
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


    // 4. Endgültiges Ergebnis aus dem Mutex extrahieren
    // Arc::try_unwrap gibt das innere `Mutex` zurück, wenn alle Arcs gelöscht sind
    let final_best_hydrant = Arc::try_unwrap(best_hydrant_arc)
        .ok()
        .expect("Interner Fehler: Arc sollte der einzige Strong Count sein, nachdem alle Handles beendet wurden.")
        .into_inner()
        .expect("Interner Fehler: Mutex ist vergiftet (Task Panik in der Sperre).");


    final_best_hydrant.ok_or_else(|| anyhow::anyhow!("❌ Keine gültige Route zu Hydranten gefunden."))
}

/// Ruft eine statische Karte von der Stadia Maps API ab, die die Route anzeigt.
///
/// Die Route und die Positionen werden im Request-Body als JSON übermittelt.
///
/// # Arguments
///
/// * `user` - Die Position des Benutzers (roter Marker).
/// * `hydrant` - Der beste Hydrant (blauer Marker mit 'H').
/// * `width` - Die gewünschte Breite der Karte in Pixeln.
/// * `height` - Die gewünschte Höhe der Karte in Pixeln.
///
/// # Returns
///
/// Die Bilddaten der Karte als `Vec<u8>` (PNG-Format).
pub async fn get_map(user: &Location, hydrant: &RankedHydrant, width: u32, height: u32) -> Result<Vec<u8>> {
    let api_key = env::var("STADIAMAPS_API_KEY")
        .context("❌ STADIAMAPS_API_KEY Umgebungsvariable muss gesetzt sein. Kartenerstellung nicht möglich.")?;

    let hydrant_loc = &hydrant.hydrant;
    let polyline_param = &hydrant.route_details.geometry_polyline;
    let zoom = calculate_zoom_level(user, hydrant_loc);


    let request_body = MapRequest {
        center: format!(
            "{},{}",
            (user.latitude + hydrant_loc.latitude) / 2f64,
            (user.longitude + hydrant_loc.longitude) / 2f64,
        ),
        size: format!("{}x{}@2x", width, height),
        zoom,
        markers: vec![
            MapMarker {
                lat: user.latitude,
                lon: user.longitude,
                color: Some("red".to_string()),
                label: None,
            },
            MapMarker {
                lat: hydrant_loc.latitude,
                lon: hydrant_loc.longitude,
                color: Some("blue".to_string()),
                label: Some("H".to_string()),
            },
        ],
        lines: if polyline_param.is_empty() {
            vec![]
        } else {
            vec![
                MapLine {
                    shape: polyline_param.clone(),
                    stroke_color: Some("red".to_string()),
                    stroke_width: Some(5),
                }
            ]
        },
    };

    let url = format!(
        "https://tiles.stadiamaps.com/static_cacheable/alidade_smooth.png?api_key={}",
        api_key
    );

    let client = reqwest::Client::new();

    println!("{:?}", request_body);

    let response = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&request_body)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        // Versuch, den Fehler-Body abzurufen. Ungültige/Fehlende Bodies werden ignoriert,
        // um keinen doppelten Fehler beim Logging zu erzeugen.
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "Kein Antwort-Body verfügbar".to_string());

        // Fehler auslösen, wenn der Status nicht erfolgreich ist.
        anyhow::bail!(
            "🖼️ Stadia Maps API Fehler (Status: {}). Antwort: {}",
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
/// Der berechnete Zoomlevel als `u32`.
pub fn calculate_zoom_level(loc1: &Location, loc2: &Location) -> u32 {
    let mid_lat_rad = (loc1.latitude.to_radians() + loc2.latitude.to_radians()) / 2.0;
    let max_lon_dist = (loc1.longitude - loc2.longitude).abs();

    // Näherungsweise Berechnung der Meter pro Längengrad auf der mittleren Breite
    let meters_per_lon_degree = 6371000.0 * 2.0 * PI * mid_lat_rad.cos() / 360.0;
    let horizontal_distance_m = max_lon_dist * meters_per_lon_degree;
    if horizontal_distance_m == 0.0 {
        return MAX_ZOOM;
    }

    let zoom_factor = 450.0; // Faktor, der die Auflösung der Kachelgröße berücksichtigt
    let zoom_f64 = MAP_WIDTH_PIXELS.log2() + (zoom_factor / horizontal_distance_m).log2();

    let final_zoom = zoom_f64.floor() as u32;

    final_zoom.clamp(0, MAX_ZOOM)
}


pub async fn retry_on_failure<T, F, Fut>(f: F, max_retries: u32, delay: Duration,) -> Result<T> where F: Fn() -> Fut, Fut: std::future::Future<Output = Result<T>>,{
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
                    return Err(e).context(format!("API-Aufruf fehlgeschlagen: Permanenter Fehler."));
                }

                // 2. Ist es ein transienter Fehler UND der letzte Versuch? -> Abbrechen nach Retries.
                if attempt >= max_retries {
                    // Letzter Versuch fehlgeschlagen: Fehler ausgeben und beenden
                    anyhow::bail!("❌ API-Aufruf nach {} Versuchen fehlgeschlagen. Letzter Fehler: {}", max_retries, e);
                }

                // 3. Transienter Fehler und weitere Versuche übrig -> Warten und fortsetzen.
                println!("⚠️ Versuch {}/{}: API fehlgeschlagen. Warte {}s...", attempt, max_retries, delay.as_secs_f32());
                sleep(delay).await;

                // Zähler erhöhen und die `loop` startet automatisch den nächsten Versuch
                attempt += 1;
            }
        }
    }
}

fn is_transient_error(e: &anyhow::Error) -> bool {
    // Versuche, den Fehler in einen reqwest::Error umzuwandeln
    if let Some(reqwest_error) = e.downcast_ref::<reqwest::Error>() {
        // Prüfe auf HTTP Status Code Fehler
        if let Some(status) = reqwest_error.status() {
            return status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS;
        }

        // Prüfe auf Netzwerkfehler/Timeouts
        return reqwest_error.is_timeout() || reqwest_error.is_connect();
    }

    // Wenn es kein reqwest-Fehler ist oder der Typ unbekannt ist, behandeln wir ihn als fatal
    false
}