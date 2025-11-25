use serde::{Deserialize, Serialize};


/// --- API / Overpass Structs ---

/// Repräsentiert die gesamte JSON-Antwort von der Overpass API.
#[derive(Debug, Deserialize)]
pub struct LocationResponse {
    /// Die Liste der von Overpass gefundenen geographischen Elemente (z.B. Hydranten).
    pub elements: Vec<Location>,
}


/// --- API / OSRM Structs ---

/// Repräsentiert die Geometrie eines GeoJSON-Objekts (verwendet von ORS und OSRM).
#[derive(Debug, Deserialize)]
pub struct GeoJsonGeometry {
    /// Die Liste der Koordinatenpunkte der Geometrie (Array von [Länge, Breite]-Paaren).
    pub coordinates: Vec<Vec<f64>>,
    /// Der Typ der GeoJSON-Geometrie (z.B. "LineString").
    #[serde(rename = "type")]
    pub geo_type: String,
}

/// Repräsentiert ein einzelnes Segment (Leg) einer Route in der OSRM-Antwort.
#[derive(Debug, Deserialize)]
pub struct RouteLeg {
    /// Die Distanz des Segments in Metern.
    pub distance: f64,
}

/// Repräsentiert eine berechnete Route in der OSRM-Antwort.
#[derive(Debug, Deserialize)]
pub struct Route {
    /// Die Segmente der Route.
    pub legs: Vec<RouteLeg>,
    /// Die Geometrie der Route, kodiert als Polyline String (mit Standard-Präzision).
    pub geometry: String,
}

/// Repräsentiert die gesamte JSON-Antwort von der OSRM API.
#[derive(Debug, Deserialize)]
pub struct OSRMResponse {
    /// Die Liste der gefundenen Routen (meist nur eine).
    pub routes: Vec<Route>,
}


/// --- API / OpenRouteService (ORS) Structs ---

/// Eine Zusammenfassung der Routendetails in der ORS-Antwort.
#[derive(Debug, Deserialize)]
pub struct RouteSummary {
    /// Die Distanz der Route in Metern.
    pub distance: f64,
}

/// Die Eigenschaften (Properties) eines Features in der ORS-Antwort.
#[derive(Debug, Deserialize)]
pub struct ORSProperties {
    /// Die Zusammenfassung der Route.
    pub summary: RouteSummary,
}

/// Ein geographisches Feature, das eine Route in der ORS-Antwort enthält.
#[derive(Debug, Deserialize)]
pub struct ORSFeature {
    /// Die Eigenschaften des Features, einschließlich der Routenzusammenfassung.
    pub properties: ORSProperties,
    /// Die Geometrie des Features.
    pub geometry: GeoJsonGeometry,
}

/// Repräsentiert die gesamte JSON-Antwort von der OpenRouteService (ORS) API.
#[derive(Debug, Deserialize)]
pub struct ORSResponse {
    /// Die Liste der Features (Routen), meist nur ein Element.
    pub features: Vec<ORSFeature>,
}


/// --- Interne / Gemeinsame Datenstrukturen ---

/// Repräsentiert eine geographische Position (Breite und Länge).
///
/// Wird für die Position des Benutzers und die Position des Hydranten verwendet.
/// Die Felder werden für die Serialisierung/Deserialisierung mit Overpass (`lon`/`lat`) angepasst.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Location {
    /// Die Länge (Longitude) der Position. Wird von `serde` von "lon" umbenannt.
    #[serde(rename = "lon")]
    pub longitude: f64,
    /// Die Breite (Latitude) der Position. Wird von `serde` von "lat" umbenannt.
    #[serde(rename = "lat")]
    pub latitude: f64,
    /// Optionale Tags, falls die Position ein OSM-Element mit Tags (wie ein Hydrant) ist.
    #[serde(rename = "tags")]
    pub tags: Option<HydrantTags>,
}

/// Details einer berechneten Route zwischen zwei Punkten.
#[derive(Debug, Serialize, Deserialize)]
pub struct RouteDetails {
    /// Die berechnete Distanz der Route in Metern (z.B. Fußweglänge).
    pub distance_meters: f64,
    /// Die Route selbst, kodiert als Polyline String (z.B. für Stadia Maps). Kann leer sein, wenn Fallback auf Haversine.
    pub geometry_polyline: String,
}

/// Repräsentiert die Form (Bauart) des Hydranten.
#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub enum HydrantShape {
    #[serde(rename = "pipe")]
    Pipe,
    #[serde(rename = "pillar")]
    Pillar, // Überflurhydrant
    #[serde(rename = "underground")]
    Underground, // Unterflurhydrant
    #[serde(rename = "wall")]
    Wall,
}

impl HydrantShape {
    /// Liefert die benutzerfreundliche, deutsche Beschreibung der Form.
    pub fn to_display_name(&self) -> &'static str {
        match self {
            HydrantShape::Pipe => "Rohrhydrant",
            HydrantShape::Pillar => "Überflurhydrant",
            HydrantShape::Wall => "Wandhydrant",
            HydrantShape::Underground => "Unterflurhydrant",
        }
    }
}

/// Repräsentiert die genaue Position des Hydranten im Straßenraum.
#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub enum HydrantPosition {
    #[serde(rename = "green")]
    Green, // Grünfläche
    #[serde(rename = "lane")]
    Lane, // Straße
    #[serde(rename = "parking_lot")]
    ParkingLot, // Parkplatz
    #[serde(rename = "sidewalk")]
    Sidewalk, // Gehweg
}

impl HydrantPosition {
    /// Liefert die benutzerfreundliche, deutsche Beschreibung der Position.
    pub fn to_display_name(&self) -> &'static str {
        match self {
            HydrantPosition::Green => "Grünfläche",
            HydrantPosition::Lane => "Straße",
            HydrantPosition::ParkingLot => "Parkplatz",
            HydrantPosition::Sidewalk => "Gehweg",
        }
    }
}

/// Die von der Overpass API zurückgegebenen Tags für einen Hydranten.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HydrantTags {
    /// Die Bauart des Hydranten (`fire_hydrant:type`).
    #[serde(rename = "fire_hydrant:type")]
    pub shape: Option<HydrantShape>,

    /// Die genaue Position im Straßenraum (`fire_hydrant:position`).
    #[serde(rename = "fire_hydrant:position")]
    pub position: Option<HydrantPosition>,

    /// Der Durchmesser des Hydrantenrohres in Millimetern (`fire_hydrant:diameter`).
    #[serde(rename = "fire_hydrant:diameter")]
    pub diameter: Option<String>,
}

/// Ein Hydrant, der nach seiner Routendistanz zum Benutzer gerankt wurde.
#[derive(Debug)]
pub struct RankedHydrant {
    /// Die geographische Position des Hydranten.
    pub hydrant: Location,
    /// Die Routendetails vom Benutzer zu diesem Hydranten.
    pub route_details: RouteDetails,
}

/// --- API / Stadia Maps Static Map Request Structs ---

/// Repräsentiert einen einzelnen Marker (Punkt) auf der statischen Karte.
#[derive(Debug, Serialize)]
pub struct MapMarker {
    /// Die Breite (Latitude) des Markers.
    pub lat: f64,
    /// Die Länge (Longitude) des Markers.
    pub lon: f64,
    /// Optionale Farbe des Markers (z.B. "red", "blue").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Optionales Label/Textzeichen auf dem Marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Repräsentiert eine Linie (z.B. eine Route) auf der statischen Karte.
#[derive(Debug, Serialize)]
pub struct MapLine {
    /// Die Form (Shape) der Linie, kodiert als Polyline String.
    pub shape: String,
    /// Optionale Farbe der Linie (z.B. "red").
    #[serde(rename = "color", skip_serializing_if = "Option::is_none")]
    pub stroke_color: Option<String>,
    /// Optionale Dicke der Linie in Pixeln.
    #[serde(rename = "width", skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<u32>,
}

/// Repräsentiert die gesamte JSON-Anfrage (POST-Body) an die Stadia Maps Static API.
#[derive(Debug, Serialize)]
pub struct MapRequest {
    /// Die Koordinaten des Kartenzentrums (z.B. "lat,lon").
    pub center: String,
    /// Die Größe der generierten Karte (z.B. "800x600@2x").
    pub size: String,
    /// Der Zoomlevel der Karte.
    pub zoom: u32,
    /// Die Liste der Linien (Routen), die auf der Karte gezeichnet werden sollen.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<MapLine>,
    /// Die Liste der Marker (Punkte), die auf der Karte gezeichnet werden sollen.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<MapMarker>,
}


impl Location {
    /// Erstellt eine neue `Location` Instanz ohne Hydranten-Tags.
    ///
    /// # Arguments
    ///
    /// * `lon` - Die Länge (Longitude).
    /// * `lat` - Die Breite (Latitude).
    pub fn new(lon: f64, lat: f64) -> Self {
        Location {
            longitude: lon,
            latitude: lat,
            tags: None,
        }
    }

}

impl HydrantTags {
    pub fn output(&self){
        if let Some(shape) = &self.shape {
            println!("🧯 Es handelt sich um einen {}", shape.to_display_name());
        }
        if let Some(diameter) = &self.diameter {
            println!("🧯 Das Rohr hat einen Durchmesser von {}mm", diameter);
        }
        if let Some(position) = &self.position {
            match position {
                HydrantPosition::Green => println!("🧯 Der Hydrant befindet sich auf sich einer Grünfläche."),
                HydrantPosition::Lane => println!("🧯 Der Hydrant befindet sich auf der Straße."),
                HydrantPosition::ParkingLot => println!("🧯 Der Hydrant befindet sich auf einem Parkplatz."),
                HydrantPosition::Sidewalk => println!("🧯 Der Hydrant befindet sich auf dem Gehweg."),
            }
        }

    }

}