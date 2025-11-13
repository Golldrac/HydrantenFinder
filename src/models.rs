use serde::{Deserialize, Serialize};



// --- API / Overpass Structs ---
#[derive(Debug, Deserialize)]
pub struct LocationResponse {
    pub elements: Vec<Location>,
}


// --- API / OSRM Structs ---
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


// --- API / OpenRouteService (ORS) Structs ---
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


// --- Interne / Gemeinsame Datenstrukturen ---
#[derive(Debug, Serialize, Deserialize, Clone)] 
pub struct Location {
    #[serde(rename = "lon")]
    pub longitude: f64,
    #[serde(rename = "lat")]
    pub latitude: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RouteDetails {
    pub distance_meters: f64,
    pub geometry_polyline: String,
}

#[derive(Debug)]
pub struct RankedHydrant {
    pub location: Location,
    pub route_details: RouteDetails,
}

// --- API / Stadia Maps Static Map Request Structs ---

#[derive(Debug, Serialize)]
pub struct MapMarker {
    pub lat: f64,
    pub lon: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MapLine {
    pub shape: String,
    #[serde(rename = "color", skip_serializing_if = "Option::is_none")]
    pub stroke_color: Option<String>,
    #[serde(rename = "width", skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<u32>,
}


#[derive(Debug, Serialize)]
pub struct MapRequest {
    pub center: String,
    pub size: String,
    pub zoom: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<MapLine>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<MapMarker>,
}


impl Location {
    pub fn new(lon: f64, lat: f64) -> Self {
        Location {
            longitude: lon,
            latitude: lat,
        }
    }

}