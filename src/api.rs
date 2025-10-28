use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Coordinate {
    #[serde(rename = "lon")]
    pub longitude: f64,
    #[serde(rename = "lat")]
    pub latitude: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoordinateResponse {
    pub elements: Vec<Coordinate>,
}


pub async fn find_hydrants(lat: f64, lon: f64, radius_m: u32,) -> Result<Vec<Coordinate>, Box<dyn std::error::Error>> {

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

    Ok(response.elements)
}