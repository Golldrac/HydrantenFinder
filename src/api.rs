use geo::{HaversineDistance, Point};
use serde::{Deserialize, Serialize};
use std::env;
use std::error::Error;
use serde_json;
use dotenvy;


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
}

#[derive(Debug, Deserialize)]
pub struct ORSResponse {
    pub features: Vec<ORSFeature>, // Erwartet jetzt "features" statt "routes"
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Coordinate {
    #[serde(rename = "lon")]
    pub longitude: f64,
    #[serde(rename = "lat")]
    pub latitude: f64,
}
impl Coordinate {
    pub fn new(lon: f64, lat: f64) -> Self {
        Coordinate {
            longitude: lon,
            latitude: lat,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoordinateResponse {
    pub elements: Vec<Coordinate>,
}

pub async fn find_hydrants(location: &Coordinate, radius_m: u32) -> Result<Vec<Coordinate>, Box<dyn std::error::Error>> {
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
        .await?; // 1. Rohe HTTP-Antwort speichern

    // NEU: Fehlerbehandlung vor JSON-Parsing
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_else(|_| "Kein Antwort-Body verfügbar".to_string());

        // Overpass gibt oft Klartext-Fehler zurück (z.B. bei Serverauslastung).
        return Err(format!("Overpass API Fehler. Statuscode: {}. Antwort: {}", status, body).into());
    }

    // 2. Wenn der Status erfolgreich ist, versuche zu deserialisieren
    let response_body = response.json::<CoordinateResponse>()
        .await?;

    println!("Alle Hydranten bekommen: {}", response_body.elements.len());

    Ok(response_body.elements)
}
pub async fn get_nearest_hydrants(location: &Coordinate, amount: Option<usize>) -> Vec<Coordinate> {
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

    let mut nearest_hydrants: Vec<(Coordinate, f64)> = hydrant_locations
        .into_iter()
        .map(|hydrant| {
            let hydrant_point = Point::new(hydrant.longitude, hydrant.latitude);
            let distance = user_location_point.haversine_distance(&hydrant_point);
            (hydrant, distance)
        })
        .collect();

    nearest_hydrants.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let final_coords: Vec<Coordinate> = nearest_hydrants
        .into_iter()
        .take(max_len)
        .map(|(coord, _distance)| coord)
        .collect();

    return final_coords;
}

//TODO: Den weg zurüchgeben und nicht nur die distanz
//TODO: erst get_route_distance_1, wennn das keine route findet, get_route_distance!!

pub async fn get_route_distance(start: &Coordinate, destination: &Coordinate) -> Result<f64, Box<dyn std::error::Error>> {

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

    // 1. HTTP-Status prüfen (fängt 4xx/5xx ab)
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_else(|_| "Kein Antwort-Body verfügbar".to_string());

        return Err(format!("ORS API Fehler. Statuscode: {}. Antwort: {}", status, body).into());
    }

    // 2. Antwortkörper als Text lesen (konsumiert die Antwort)
    let body_text = response.text().await?;

    // 3. Antwortkörper manuell deserialisieren. Dies fängt den Fehler "missing field routes" ab.
    let response_body: ORSResponse = match serde_json::from_str(&body_text) {
        Ok(body) => body,
        Err(e) => {
            // Dies ist der entscheidende Teil: Wir geben den Deserialisierungsfehler UND den rohen Body aus
            return Err(format!("ORS Decode Fehler: {}. Ungültige JSON-Antwort (fehlt 'routes' trotz 200 OK?): {}", e, body_text).into());
        }
    };

    println!("ORS Response erhalten: {:?}", response_body);


    if let Some(feature) = response_body.features.into_iter().next() {
        // Der Pfad zur Distanz ist jetzt: feature.properties.summary.distance
        let route_distance = feature.properties.summary.distance;

        // Fallback-Logik für Snapping-Fehler beibehalten
        if route_distance == 0.0 {
            // ... (Ihre Fallback-Logik für 0m-Distanz bleibt hier unverändert)
            let start_point = Point::new(start.longitude, start.latitude);
            let dest_point = Point::new(destination.longitude, destination.latitude);
            let haversine_dist = start_point.haversine_distance(&dest_point);

            if haversine_dist > 10.0 {
                eprintln!("⚠️ ORS Route war 0m, aber Luftlinie ist {:.2}m. Wahrscheinlich Snapping-Fehler. Rückgabe Luftlinie.", haversine_dist);
                return Ok(haversine_dist);
            }
        }

        return Ok(route_distance);
    }

    // Der Fehlertext ist jetzt genauer, falls keine Route gefunden wird
    Err("Keine Route (Feature) in der ORS-Antwort gefunden.".into())
}

/*
pub async fn get_route_distance_1(start: &Coordinate, destination: &Coordinate) -> Result<f64, Box<dyn std::error::Error>> {
    let url = format!(
        "http://router.project-osrm.org/route/v1/foot/{},{};{},{}?steps=false&snapping=any&overview=false",
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
        println!("{:?}", route);
        if let Some(leg) = route.legs.into_iter().next() {
            return Ok(leg.distance);
        }
    }

    Err("Keine Route gefunden oder API-Antwort fehlerhaft".into())
}
 */