mod utils;

use std::f64::consts::PI;
use anyhow::{Context, Result};
use futures::future::join_all;
use geo::{HaversineDistance, Point};
use geo_types::Coordinate as GeoTypesCoordinate;
use polyline;
use serde_json;
use std::env;

use crate::POLYLINE_PRECISION;
use crate::models::{
    Location, LocationResponse, ORSResponse, OSRMResponse, RankedHydrant, RouteDetails, MapRequest, MapMarker, MapLine,
};

const PROXIMITY_THRESHOLD_METERS: f64 = 5.0;
const MAP_WIDTH_PIXELS: f64 = 800.0;
const MAX_ZOOM: u32 = 18;

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

    let url = "http://overpass-api.de/api/interpreter";

    let client = reqwest::Client::new();
    let response = client.post(url).body(query).send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .context("Kein Antwort-Body verfügbar")?;
        anyhow::bail!(
            "Overpass API Fehler. Statuscode: {}. Antwort: {}",
            status,
            body
        );
    }

    let response_body = response.json::<LocationResponse>().await?;

    println!("Alle Hydranten bekommen: {}", response_body.elements.len());

    Ok(response_body.elements)
}

pub async fn get_nearest_hydrants(location: &Location, amount: Option<usize>,) -> Result<Vec<Location>> {
    let max_len = amount.unwrap_or(5);

    let coordinates_result = find_hydrants(location, 500).await;

    let hydrant_locations = match coordinates_result {
        Ok(locs) => locs,
        Err(e) => {
            anyhow::bail!("Fehler beim Abruf der Hydranten: {}", e);
        }
    };

    if hydrant_locations.is_empty() {
        anyhow::bail!("Keine Hydranten in der Nähe gefunden");
    }

    println!("hydrant_locations nicht leer");

    let user_location_point = Point::new(location.longitude, location.latitude);

    let mut nearest_hydrants: Vec<(Location, f64)> = hydrant_locations
        .into_iter()
        .map(|hydrant| {
            let hydrant_point = Point::new(hydrant.longitude, hydrant.latitude);
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

    return Ok(final_coords);
}

pub async fn fetch_route_osrm(start: &Location, destination: &Location) -> Result<RouteDetails> {
    let url = format!(
        "http://router.project-osrm.org/route/v1/foot/{},{};{},{}?steps=false&snapping=any&overview=full",
        start.longitude, start.latitude, destination.longitude, destination.latitude
    );

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .send()
        .await?
        .json::<OSRMResponse>()
        .await?;

    if let Some(route) = response.routes.into_iter().next() {
        if let Some(leg) = route.legs.into_iter().next() {
            if leg.distance == 0.0 {
                return fetch_route_ors(start, destination).await;
            }
            return Ok(RouteDetails {
                distance_meters: leg.distance,
                geometry_polyline: route.geometry,
            });
        }
    }

    fetch_route_ors(start, destination).await
}

pub async fn fetch_route_ors(start: &Location, destination: &Location) -> Result<RouteDetails> {
    //Wenn keine Abfragen mehr übrig sind, neuer Account mit neuem Key erstellen
    let api_key =
        env::var("ORS_API_KEY").context("ORS_API_KEY Umgebungsvariable muss gesetzt sein.")?;

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
            .unwrap_or_else(|_| "Kein Antwort-Body verfügbar".to_string());
        anyhow::bail!("ORS API Fehler. Statuscode: {}. Antwort: {}", status, body);
    }

    let body_text = response.text().await?;

    let response_body: ORSResponse = match serde_json::from_str(&body_text) {
        Ok(body) => body,
        Err(e) => {
            anyhow::bail!(
                "ORS Decode Fehler: {}. Ungültige JSON-Antwort: {}",
                e,
                body_text
            );
        }
    };

    if let Some(feature) = response_body.features.into_iter().next() {
        let route_distance = feature.properties.summary.distance;
        let coords = feature.geometry.coordinates;

        let geo_coords: Vec<GeoTypesCoordinate<f64>> = coords
            .into_iter()
            .map(|c| GeoTypesCoordinate { x: c[0], y: c[1] })
            .collect();

        let geometry_polyline = polyline::encode_coordinates(geo_coords, POLYLINE_PRECISION)?;

        if route_distance == 0.0 {
            let start_point = Point::new(start.longitude, start.latitude);
            let dest_point = Point::new(destination.longitude, destination.latitude);
            let haversine_dist = start_point.haversine_distance(&dest_point);

            if haversine_dist > 10.0 {
                eprintln!(
                    "⚠️ ORS Route war 0m, aber Luftlinie ist {:.2}m. Wahrscheinlich Snapping-Fehler. Rückgabe Luftlinie.",
                    haversine_dist
                );
                return Ok(RouteDetails {
                    distance_meters: haversine_dist,
                    geometry_polyline: "".to_string(),
                });
            }
        }

        return Ok(RouteDetails {
            distance_meters: route_distance,
            geometry_polyline,
        });
    }

    anyhow::bail!("Keine Route (Feature) in der ORS-Antwort gefunden.");
}

pub async fn get_closest_by_route(user_location: &Location, candidate_hydrants: &Vec<Location>,) -> Result<RankedHydrant> {
    if candidate_hydrants.is_empty() {
        anyhow::bail!("Keine Hydranten-Kandidaten zur Routenberechnung vorhanden.");
    }

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

    let route_futures: Vec<_> = candidate_hydrants
        .iter()
        .map(|hydrant_location| async move {
            let route_result = fetch_route_osrm(user_location, &hydrant_location).await;

            match route_result {
                Ok(details) => Ok(RankedHydrant {
                    location: Location::new(hydrant_location.longitude, hydrant_location.latitude),
                    route_details: details,
                }),
                Err(e) => Err(format!(
                    "Fehler beim Routing zu ({}, {}): {}",
                    hydrant_location.latitude, hydrant_location.longitude, e
                )),
            }
        })
        .collect();

    let results: Vec<Result<RankedHydrant, String>> = join_all(route_futures).await;

    let best_hydrant = results
        .into_iter()
        .filter_map(Result::ok)
        .min_by_key(|h| (h.route_details.distance_meters * 100.0) as i64);

    match best_hydrant {
        Some(h) => Ok(h),
        None => anyhow::bail!("Keine gültige Route unter den Kandidaten gefunden."),
    }
}

pub async fn get_map_post(user: &Location, hydrant: &RankedHydrant, width: u32, height: u32,) -> Result<Vec<u8>> {
    let api_key = env::var("STADIAMAPS_API_KEY")
        .context("STADIAMAPS_API_KEY Umgebungsvariable muss gesetzt sein.")?;

    ////Wenn keine Abfragen mehr übrig sind, neuer Account mit neuem Key erstellen

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
            .context("Kein Antwort-Body verfügbar")?;
        anyhow::bail!(
            "Stadia Maps API (POST) Fehler. Statuscode: {}. Antwort: {}",
            status,
            body
        );
    }

    Ok(response.bytes().await?.to_vec())
}
pub fn calculate_zoom_level(loc1: &Location, loc2: &Location) -> u32 {
    let mid_lat_rad = (loc1.latitude.to_radians() + loc2.latitude.to_radians()) / 2.0;
    let max_lon_dist = (loc1.longitude - loc2.longitude).abs();

    let meters_per_lon_degree = 6371000.0 * 2.0 * PI * mid_lat_rad.cos() / 360.0;
    let horizontal_distance_m = max_lon_dist * meters_per_lon_degree;
    if horizontal_distance_m == 0.0 {
        return MAX_ZOOM;
    }

    let zoom_factor = 450.0;
    let zoom_f64 = MAP_WIDTH_PIXELS.log2() + (zoom_factor / horizontal_distance_m).log2();

    let final_zoom = zoom_f64.floor() as u32;

    final_zoom.clamp(0, MAX_ZOOM)
}