use geo::{HaversineDistance, Point};
use geo_types::Coordinate as GeoTypesCoordinate;
use serde::{Deserialize, Serialize};
use std::env;
use serde_json;
use dotenvy;
use polyline;



#[derive(Debug, Deserialize)]
pub struct GeoJsonGeometry {
    pub coordinates: Vec<Vec<f64>>,
    #[serde(rename = "type")]
    pub geo_type: String,
}
#[derive(Debug, Deserialize)]
pub struct RouteLeg {
    pub distance: f64,
}

#[derive(Debug, Deserialize)]
pub struct Route {
    pub legs: Vec<RouteLeg>,
    pub geometry: String,
}

#[derive(Debug, Deserialize)]
pub struct OSRMResponse {
    pub routes: Vec<Route>,
}

#[derive(Debug, Deserialize)]
pub struct RouteSummary {
    pub distance: f64,
}

#[derive(Debug, Deserialize)]
pub struct ORSProperties {
    pub summary: RouteSummary,
}

#[derive(Debug, Deserialize)]
pub struct ORSFeature {
    pub properties: ORSProperties,
    pub geometry: GeoJsonGeometry,
}

#[derive(Debug, Deserialize)]
pub struct ORSResponse {
    pub features: Vec<ORSFeature>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RouteDetails {
    pub distance_meters: f64,
    pub geometry_polyline: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Location {
    #[serde(rename = "lon")]
    pub longitude: f64,
    #[serde(rename = "lat")]
    pub latitude: f64,
}
impl Location {
    pub fn new(lon: f64, lat: f64) -> Self {
        Location {
            longitude: lon,
            latitude: lat,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LocationResponse {
    pub elements: Vec<Location>,
}

pub async fn find_hydrants(location: &Location, radius_m: u32) -> Result<Vec<Location>, Box<dyn std::error::Error>> {
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
    let response = client
        .post(url)
        .body(query)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_else(|_| "Kein Antwort-Body verfügbar".to_string());

        return Err(format!("Overpass API Fehler. Statuscode: {}. Antwort: {}", status, body).into());
    }

    let response_body = response.json::<LocationResponse>()
        .await?;

    println!("Alle Hydranten bekommen: {}", response_body.elements.len());

    Ok(response_body.elements)
}

pub async fn get_nearest_hydrants(location: &Location, amount: Option<usize>) -> Vec<Location> {
    let max_len = amount.unwrap_or(5);

    let coordinates_result = find_hydrants(location, 500).await;

    let hydrant_locations = match coordinates_result {
        Ok(locs) => locs,
        Err(e) => {
            eprintln!("Fehler beim Abruf der Hydranten: {}", e);
            return Vec::new();
        }
    };

    if hydrant_locations.is_empty() {
        return Vec::new();
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

    return final_coords;
}

pub async fn fetch_route_osrm(start: &Location, destination: &Location) -> Result<RouteDetails, Box<dyn std::error::Error>> {
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

    println!("OSRM Response erhalten: {:?}", response);

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

pub async fn fetch_route_ors(start: &Location, destination: &Location) -> Result<RouteDetails, Box<dyn std::error::Error>> {

    let api_key = env::var("ORS_API_KEY").unwrap_or_else(|_| {
        eprintln!("API_KEY nicht gesetzt, verwende Standardwert.");
        "default_key".to_string()
    });

    let url = format!(
        "https://api.openrouteservice.org/v2/directions/foot-walking?api_key={}&start={},{}&end={},{}&snapping=any",
        api_key,
        start.longitude, start.latitude,
        destination.longitude, destination.latitude
    );

    let client = reqwest::Client::new();

    let response = client
        .get(&url)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_else(|_| "Kein Antwort-Body verfügbar".to_string());

        return Err(format!("ORS API Fehler. Statuscode: {}. Antwort: {}", status, body).into());
    }

    let body_text = response.text().await?;

    let response_body: ORSResponse = match serde_json::from_str(&body_text) {
        Ok(body) => body,
        Err(e) => {
            return Err(format!("ORS Decode Fehler: {}. Ungültige JSON-Antwort: {}", e, body_text).into());
        }
    };

    println!("ORS Response erhalten: {:?}", response_body);


    if let Some(feature) = response_body.features.into_iter().next() {
        let route_distance = feature.properties.summary.distance;
        let coords = feature.geometry.coordinates;

        let geo_coords: Vec<GeoTypesCoordinate<f64>> = coords.into_iter()
            .map(|c| {
                GeoTypesCoordinate { x: c[0], y: c[1] }
            })
            .collect();

        let geometry_polyline = polyline::encode_coordinates(geo_coords, 5)?;

        if route_distance == 0.0 {
            let start_point = Point::new(start.longitude, start.latitude);
            let dest_point = Point::new(destination.longitude, destination.latitude);
            let haversine_dist = start_point.haversine_distance(&dest_point);

            if haversine_dist > 10.0 {
                eprintln!("⚠️ ORS Route war 0m, aber Luftlinie ist {:.2}m. Wahrscheinlich Snapping-Fehler. Rückgabe Luftlinie.", haversine_dist);
                return Ok(RouteDetails { distance_meters: haversine_dist, geometry_polyline: "".to_string() });
            }
        }

        return Ok(RouteDetails {
            distance_meters: route_distance,
            geometry_polyline,
        });
    }

    Err("Keine Route (Feature) in der ORS-Antwort gefunden.".into())
}

pub fn decode_route_polyline(polyline_str: &str, precision: u32) -> Result<Vec<Location>, String> {

    let decoded_coords = polyline::decode_polyline(polyline_str, precision)
        .map_err(|e| format!("Fehler beim Dekodieren der Polyline: {:?}", e))?;

    let locations: Vec<Location> = decoded_coords.into_iter()
        .map(|coord| Location {
            longitude: coord.x,
            latitude: coord.y,
        })
        .collect();

    Ok(locations)
}