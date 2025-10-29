use geo::{HaversineDistance, Point};
use serde::{Deserialize, Serialize};

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

pub async fn find_hydrants(location: &Coordinate, radius_m: u32,) -> Result<Vec<Coordinate>, Box<dyn std::error::Error>> {
    let lat = location.latitude;
    let lon = location.longitude;
    let query = format!(
        r#"[out:json][timeout:25];
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
        .await?
        .json::<CoordinateResponse>()
        .await?;

    println!("Alle Hydranten bekommen: {}", response.elements.len());

    Ok(response.elements)
}
pub async fn get_nearest_hydrants(location: Coordinate, amount: Option<usize>) -> Vec<Coordinate> {
    let max_len = amount.unwrap_or(5);

    let coordinates_result = find_hydrants(&location, 500).await;

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