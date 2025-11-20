use std::f64::consts::PI;
use std::env;

use anyhow::{Context, Result};
use futures::future::join_all;
use geo::{Haversine, HaversineDistance, Point};
use geo_types::Coord;
use polyline;
use reqwest;
use serde_json;

use crate::models::{
    Location, LocationResponse, MapLine, MapMarker, MapRequest, ORSResponse, OSRMResponse,
    RankedHydrant, RouteDetails,
};

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
        out center;"#,
        radius_m, lat, lon
    );

    let url = "https://overpass-api.de/api/interpreter";

    let client = reqwest::Client::new();
    let response = client.post(url).body(query).send().await?;

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
/// * `amount` - Die maximale Anzahl der zurückzugebenden Hydranten.
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
/// **NEU:** Fällt nun auch bei Netzwerkfehlern oder HTTP-Fehler-Statuscodes auf `fetch_route_ors` zurück.
///
/// # Arguments
///
/// * `start` - Die Startposition.
/// * `destination` - Die Zielposition.
///
/// # Returns
///
/// Die `RouteDetails` der berechneten Route.
pub async fn fetch_route_osrm(start: &Location, destination: &Location) -> Result<RouteDetails> {
    let url = format!(
        "https://router.project-osrm.org/route/v1/foot/{},{};{},{}?steps=false&snapping=any&overview=full",
        start.longitude, start.latitude, destination.longitude, destination.latitude
    );

    let client = reqwest::Client::new();

    // Sende die Anfrage und fange sofortige Netzwerkfehler ab.
    let response = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("⚠️ OSRM Network Error: {}. Falling back to ORS.", e);
            return fetch_route_ors(start, destination).await;
        }
    };

    // Fange HTTP-Statusfehler (z.B. 400 Bad Request oder 5xx Serverfehler) ab.
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_else(|_| "Kein Antwort-Body verfügbar".to_string());
        eprintln!(
            "⚠️ OSRM API Error (Status: {}). Antwort: {}. Falling back to ORS.",
            status,
            &body[..body.len().min(100)]
        );
        return fetch_route_ors(start, destination).await;
    }

    let response_body = response.json::<OSRMResponse>().await?;


    if let Some(route) = response_body.routes.into_iter().next() {
        if let Some(leg) = route.legs.into_iter().next() {
            if leg.distance == 0.0 {
                // Fallback, wenn OSRM 0m zurückgibt (z.B. bei Snapping-Problemen).
                eprintln!("⚠️ OSRM returned 0m distance. Falling back to ORS.");
                return fetch_route_ors(start, destination).await;
            }
            return Ok(RouteDetails {
                distance_meters: leg.distance,
                geometry_polyline: route.geometry,
            });
        }
    }

    eprintln!("⚠️ OSRM found no route or segment. Falling back to ORS.");
    // Fallback, wenn keine Route oder kein Segment gefunden wurde.
    fetch_route_ors(start, destination).await
}

/// Berechnet die Route zwischen Start und Ziel mithilfe der OpenRouteService (ORS) API.
///
/// ORS wird als Fallback für OSRM verwendet und erfordert einen API-Schlüssel.
///
/// # Arguments
///
/// * `start` - Die Startposition.
/// * `destination` - Die Zielposition.
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn die `ORS_API_KEY` Umgebungsvariable fehlt oder
/// die API einen Fehlerstatus zurückgibt.
pub async fn fetch_route_ors(start: &Location, destination: &Location) -> Result<RouteDetails> {
    let api_key =
        env::var("ORS_API_KEY").context("❌ ORS_API_KEY Umgebungsvariable muss gesetzt sein. Routing nicht möglich.")?;

    let url = format!(
        "https://api.openrouteservice.org/v2/directions/foot-walking?api_key={}&start={},{}&end={},{}&snapping=any",
        api_key, start.longitude, start.latitude, destination.longitude, destination.latitude
    );

    let client = reqwest::Client::new();
    let response = client.get(&url).send().await?;

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
                    "⚠️ ORS Route war 0m, aber Luftlinie ist {:.2}m. Rückgabe Luftlinie.",
                    haversine_dist
                );
                return Ok(RouteDetails {
                    distance_meters: haversine_dist,
                    geometry_polyline: String::new(), // Keine Polyline für Luftlinie
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
/// basierend auf der berechneten Geh-Route (fetch_route_osrm/ors).
///
/// # Arguments
///
/// * `user_location` - Die Startposition des Benutzers.
/// * `candidate_hydrants` - Eine Liste von Hydranten, die in Betracht gezogen werden sollen.
///
/// # Returns
///
/// Die `RankedHydrant` mit der kürzesten Geh-Distanz.
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
        return Ok(RankedHydrant {
            location: first_hydrant_location.clone(),
            route_details: RouteDetails {
                distance_meters: haversine_dist,
                geometry_polyline: String::new(),
            },
        });
    }

    // Parallel Routing
    let route_futures: Vec<_> = candidate_hydrants
        .iter()
        .map(|hydrant_location| async move {
            let route_result = fetch_route_osrm(user_location, hydrant_location).await;

            match route_result {
                Ok(details) => Ok(RankedHydrant {
                    location: hydrant_location.clone(),
                    route_details: details,
                }),
                // Fehler auf String mappen für join_all
                Err(e) => Err(format!(
                    "Routing-Fehler zu Hydrant: {}",
                    e
                )),
            }
        })
        .collect();

    let results: Vec<Result<RankedHydrant, String>> = join_all(route_futures).await;

    let errors: Vec<&String> = results.iter()
        .filter_map(|r| r.as_ref().err())
        .collect();

    for error in errors {
        eprintln!("⚠️ Routing-Fehler bei Kandidat: {}", error);
    }

    // Wir filtern nur erfolgreiche Routen heraus und wählen die kürzeste.
    let best_hydrant = results
        .into_iter()
        .filter_map(Result::ok)
        .min_by_key(|h| (h.route_details.distance_meters * 100.0) as i64);
    // Skalierung ist ein schneller Workaround für Fließkommavergleiche

    match best_hydrant {
        Some(h) => Ok(h),
        None => anyhow::bail!("🗺️ Keine gültige Route unter den Kandidaten gefunden."),
    }
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

    let hydrant_loc = &hydrant.location;
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

    let response = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&request_body)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .context("Stadia Maps: Kein Antwort-Body verfügbar")?;
        anyhow::bail!(
            "🖼️ Stadia Maps API Fehler (Status: {}). Antwort: {}",
            status,
            body
        );
    }

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