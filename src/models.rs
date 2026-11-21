use eframe::egui::Color32;
use serde::{Deserialize, Serialize};

// --- API / Overpass Structs ---

/// Repräsentiert die gesamte JSON-Antwort der Overpass API.
#[derive(Debug, Deserialize)]
pub struct LocationResponse {
    /// Liste der von Overpass gefundenen geographischen Elemente (z. B. Hydranten).
    pub elements: Vec<Hydrant>,
}

// --- API / OSRM Structs ---

/// Repräsentiert die Geometrie eines GeoJSON-Objekts (verwendet von ORS und OSRM).
#[derive(Debug, Deserialize)]
pub struct GeoJsonGeometry {
    /// Liste der Koordinatenpunkte der Geometrie als `[Länge, Breite]`-Paare.
    pub coordinates: Vec<Vec<f64>>,
}

/// Repräsentiert ein einzelnes Segment (Leg) einer Route in der OSRM-Antwort.
#[derive(Debug, Deserialize)]
pub struct RouteLeg {
    /// Distanz des Segments in Metern.
    pub distance: f64,
}

/// Repräsentiert eine berechnete Route in der OSRM-Antwort.
#[derive(Debug, Deserialize)]
pub struct Route {
    /// Segmente der Route.
    pub legs: Vec<RouteLeg>,
    /// Geometrie der Route, kodiert als Polyline-String (mit Standard-Präzision).
    pub geometry: String,
}

/// Repräsentiert die gesamte JSON-Antwort der OSRM API.
#[derive(Debug, Deserialize)]
pub struct OSRMResponse {
    /// Liste der gefundenen Routen (meist nur ein Element).
    pub routes: Vec<Route>,
}

// --- API / OpenRouteService (ORS) Structs ---

/// Zusammenfassung der Routendetails in der ORS-Antwort.
#[derive(Debug, Deserialize)]
pub struct RouteSummary {
    /// Gesamtdistanz der Route in Metern.
    pub distance: f64,
}

/// Eigenschaften (Properties) eines Features in der ORS-Antwort.
#[derive(Debug, Deserialize)]
pub struct ORSProperties {
    /// Zusammenfassung der Route.
    pub summary: RouteSummary,
}

/// Geographisches Feature, das eine Route in der ORS-Antwort enthält.
#[derive(Debug, Deserialize)]
pub struct ORSFeature {
    /// Eigenschaften des Features einschließlich der Routenzusammenfassung.
    pub properties: ORSProperties,
    /// Geometrie des Features.
    pub geometry: GeoJsonGeometry,
}

/// Repräsentiert die gesamte JSON-Antwort der OpenRouteService (ORS) API.
#[derive(Debug, Deserialize)]
pub struct ORSResponse {
    /// Liste der Features (Routen), meist nur ein Element.
    pub features: Vec<ORSFeature>,
}

// --- Interne / Gemeinsame Datenstrukturen ---

/// Repräsentiert eine geographische Position (Breiten- und Längengrad).
///
/// Wird für die Position des Benutzers oder eines Hydranten verwendet.
#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub struct Location {
    /// Längengrad (Longitude) der Position.
    #[serde(rename = "lon")]
    pub longitude: f64,
    /// Breitengrad (Latitude) der Position.
    #[serde(rename = "lat")]
    pub latitude: f64,
}

/// Repräsentiert einen Hydranten inklusive Koordinaten und optionalen OSM-Tags.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Hydrant {
    /// Eindeutige OpenStreetMap-ID des Knotens.
    #[serde(rename = "id")]
    pub osm_id: u64,
    /// Längengrad (Longitude) der Position.
    #[serde(rename = "lon")]
    pub longitude: f64,
    /// Breitengrad (Latitude) der Position.
    #[serde(rename = "lat")]
    pub latitude: f64,
    /// Optionale Metadaten-Tags des Hydranten aus der Overpass API.
    #[serde(rename = "tags")]
    pub tags: Option<HydrantTags>,
}

/// Von der Overpass API zurückgegebene OpenStreetMap-Tags für einen Hydranten.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HydrantTags {
    /// Bauart des Hydranten (`fire_hydrant:type`).
    #[serde(rename = "fire_hydrant:type")]
    pub shape: Option<HydrantShape>,
    /// Genaue Position im Straßenraum (`fire_hydrant:position`).
    #[serde(rename = "fire_hydrant:position")]
    pub position: Option<HydrantPosition>,
    /// Durchmesser des Hydrantenrohres in Millimetern (`fire_hydrant:diameter`).
    #[serde(rename = "fire_hydrant:diameter")]
    pub diameter: Option<String>,
    /// Anzahl der verfügbaren Kupplungen (`fire_hydrant:couplings`).
    #[serde(rename = "fire_hydrant:couplings")]
    pub couplings: Option<String>,
}

/// Details einer berechneten Route zwischen zwei Punkten.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RouteDetails {
    /// Berechnete Distanz der Route in Metern (z. B. Fußweglänge).
    pub distance_meters: f64,
    /// Kodierter Polyline-String der Route (leer bei Fallback auf Luftlinie).
    pub geometry_polyline: String,
}

/// Hydrant mit zugehöriger berechneter Route und Darstellungsfarbe für die GUI.
#[derive(Debug, Clone)]
pub struct HydrantWithRoute {
    /// Geographische Daten und Tags des Hydranten.
    pub hydrant: Hydrant,
    /// Routendetails vom Benutzerstandort zu diesem Hydranten.
    pub route_details: RouteDetails,
    /// Anzeigefarbe des Hydranten und seiner Route in der GUI.
    pub color: Color32,
}

// --- Enums ---

/// Bauart eines Hydranten gemäß OpenStreetMap-Tag `fire_hydrant:type`.
#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub enum HydrantShape {
    /// Rohrhydrant (`pipe`).
    #[serde(rename = "pipe")]
    Pipe,
    /// Überflurhydrant (`pillar`).
    #[serde(rename = "pillar")]
    Pillar,
    /// Unterflurhydrant (`underground`).
    #[serde(rename = "underground")]
    Underground,
    /// Wandhydrant (`wall`).
    #[serde(rename = "wall")]
    Wall,
}

/// Position eines Hydranten im Straßenraum gemäß OpenStreetMap-Tag `fire_hydrant:position`.
#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub enum HydrantPosition {
    /// Grünfläche (`green`).
    #[serde(rename = "green")]
    Green,
    /// Fahrbahn / Straße (`lane`).
    #[serde(rename = "lane")]
    Lane,
    /// Parkplatz (`parking_lot`).
    #[serde(rename = "parking_lot")]
    ParkingLot,
    /// Gehweg (`sidewalk`).
    #[serde(rename = "sidewalk")]
    Sidewalk,
}

/// Serialisierbarer Cache-Eintrag einer einzelnen Hydrantenroute für die Datenbank.
#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct CachedRouteItem {
    /// Interne Datenbank-ID des Hydranten in der Tabelle `hydrants`.
    pub hydrant_id: i32,
    /// Gespeicherte Routendistanz in Metern.
    pub distance: f64,
    /// Kodierte Routengeometrie als Polyline-String.
    pub polyline: String,
}

// --- Impl-Blöcke ---

impl Location {
    /// Erstellt eine neue [`Location`]-Instanz.
    ///
    /// # Arguments
    ///
    /// * `lat` - Der Breitengrad (Latitude).
    /// * `lon` - Der Längengrad (Longitude).
    ///
    /// # Returns
    ///
    /// Eine neue [`Location`] mit den angegebenen Koordinaten.
    pub fn new(lat: f64, lon: f64) -> Self {
        Location {
            latitude: lat,
            longitude: lon,
        }
    }
}

impl HydrantWithRoute {
    /// Erstellt eine neue [`HydrantWithRoute`]-Instanz mit Standard-Platzhalterfarbe.
    ///
    /// # Arguments
    ///
    /// * `hydrant` - Der zugehörige [`Hydrant`].
    /// * `route_details` - Die berechneten [`RouteDetails`].
    ///
    /// # Returns
    ///
    /// Eine neue [`HydrantWithRoute`]-Instanz mit [`Color32::PLACEHOLDER`].
    pub fn new(hydrant: Hydrant, route_details: RouteDetails) -> Self {
        HydrantWithRoute {
            hydrant,
            route_details,
            color: Color32::PLACEHOLDER,
        }
    }
}

impl HydrantShape {
    /// Liefert die benutzerfreundliche deutsche Bezeichnung der Hydrantenbauart.
    ///
    /// # Returns
    ///
    /// Ein statischer String-Slice mit der deutschen Bezeichnung.
    pub fn to_display_name(&self) -> &'static str {
        match self {
            HydrantShape::Pipe => "Rohrhydrant",
            HydrantShape::Pillar => "Überflurhydrant",
            HydrantShape::Wall => "Wandhydrant",
            HydrantShape::Underground => "Unterflurhydrant",
        }
    }
}