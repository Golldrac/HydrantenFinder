use serde::{Deserialize, Serialize};

// --- API / Overpass Structs ---

/// Repräsentiert die gesamte JSON-Antwort von der Overpass API.
#[derive(Debug, Deserialize)]
pub struct LocationResponse {
    /// Die Liste der von Overpass gefundenen geographischen Elemente (z.B. Hydranten).
    pub elements: Vec<Hydrant>,
}

// --- API / OSRM Structs ---

/// Repräsentiert die Geometrie eines GeoJSON-Objekts (verwendet von ORS und OSRM).
#[derive(Debug, Deserialize)]
pub struct GeoJsonGeometry {
    /// Die Liste der Koordinatenpunkte der Geometrie (Array von \[Länge, Breite]-Paaren).
    pub coordinates: Vec<Vec<f64>>,
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

// --- API / OpenRouteService (ORS) Structs ---

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

// --- API / Stadia Maps Static Map Request Structs ---

/// Repräsentiert einen einzelnen Marker (Punkt) auf der statischen Karte.
#[derive(Debug, Serialize, Clone)]
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
#[derive(Debug, Serialize, Clone)]
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
#[derive(Debug, Serialize, Clone)]
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

// --- Interne / Gemeinsame Datenstrukturen ---

/// Repräsentiert eine geographische Position (Breite und Länge).
///
/// Wird für die Position des Benutzers oder eines Hydranten verwendet.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Location {
    /// Die Länge (Longitude) der Position.
    #[serde(rename = "lon")]
    pub longitude: f64,
    /// Die Breite (Latitude) der Position.
    #[serde(rename = "lat")]
    pub latitude: f64,
}

/// Repräsentiert einen Hydranten, abgeleitet von `Location`, aber mit zusätzlichen OSM-Tags.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Hydrant {
    /// Die Länge (Longitude) der Position.
    #[serde(rename = "lon")]
    pub longitude: f64,
    /// Die Breite (Latitude) der Position.
    #[serde(rename = "lat")]
    pub latitude: f64,
    /// Optionale Tags des Hydranten, die von Overpass geliefert werden.
    #[serde(rename = "tags")]
    pub tags: Option<HydrantTags>,
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

    /// Die Anzahl der verfügbaren Kupplungen (`fire_hydrant:couplings`).
    #[serde(rename = "fire_hydrant:couplings")]
    pub couplings: Option<String>,
}

/// Details einer berechneten Route zwischen zwei Punkten.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RouteDetails {
    /// Die berechnete Distanz der Route in Metern (z.B. Fußweglänge).
    pub distance_meters: f64,
    /// Die Route selbst, kodiert als Polyline String (z.B. für Stadia Maps). Kann leer sein, wenn Fallback auf Haversine.
    pub geometry_polyline: String,
}

/// Ein Hydrant, der nach seiner Routendistanz und Eigenschaften gerankt wurde.
#[derive(Debug, Clone)]
pub struct RankedHydrant {
    /// Die geographische Position des Hydranten.
    pub hydrant: Hydrant,
    /// Die Routendetails vom Benutzer zu diesem Hydranten.
    pub route_details: RouteDetails,
    /// Der berechnete Score für die Rangliste (kleiner ist besser).
    pub score: f64,
}

// --- Enums ---

/// Repräsentiert die Form (Bauart) des Hydranten (`fire_hydrant:type`).
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

/// Repräsentiert die genaue Position des Hydranten im Straßenraum (`fire_hydrant:position`).
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

// --- Impl-Blöcke ---

impl Location {
    /// Erstellt eine neue `Location` Instanz.
    ///
    /// # Arguments
    ///
    /// * `lon` - Die Länge (Longitude).
    /// * `lat` - Die Breite (Latitude).
    pub fn new(lat: f64, lon: f64) -> Self {
        Location {
            latitude: lat,
            longitude: lon,
        }
    }
}

impl RankedHydrant {
    /// Erstellt eine neue `RankedHydrant` Instanz und berechnet sofort den Score.
    ///
    /// # Arguments
    ///
    /// * `hydrant` - Der Hydrant.
    /// * `route_details` - Die berechneten Routendetails.
    pub fn new(hydrant: Hydrant, route_details: RouteDetails) -> Self {
        let mut ranked = RankedHydrant {
            hydrant,
            route_details,
            score: 0.0, // Temporärer Wert
        };
        ranked.score = ranked.calculate_score();
        ranked
    }

    /// Berechnet den Score des Hydranten. Ein niedrigerer Score ist besser.
    ///
    /// Der Score gewichtet:
    /// 1. Route/Distanz (quadratisch): Hoher Einfluss, um kurze Strecken zu favorisieren.
    /// 2. Durchmesser: Kleiner Durchmesser (z.B. < 150mm) erhöht den Score (schlechter).
    /// 3. Typ (Shape): Überflurhydrant (`Pillar`) ist am besten (negativer Score-Add).
    /// 4. Position: Parkplatz/Grünfläche sind schlechter als Straße/Gehweg.
    /// 5. Kupplungen: Mehr als 1 Kupplung ist besser (negativer Score-Add).
    fn calculate_score(&self) -> f64 {
        // Standarddurchmesser, wenn Tag fehlt
        let diameter = if let Some(tags) = &self.hydrant.tags {
            if let Some(dia_str) = &tags.diameter {
                // Parse-Fehler ignorieren und Standardwert (80.0mm) verwenden
                dia_str.parse::<f64>().unwrap_or(80.0)
            } else {
                80.0
            }
        } else {
            80.0
        };

        // Score-Anpassung basierend auf der Bauart
        let type_add = if let Some(tags) = &self.hydrant.tags {
            match tags.shape {
                Some(HydrantShape::Pillar) => -50.0,      // Bester Typ (Überflur)
                Some(HydrantShape::Pipe) => 30.0,
                Some(HydrantShape::Wall) => 100.0,
                Some(HydrantShape::Underground) => 0.0,
                None => 0.0,
            }
        } else {
            0.0
        };

        // Score-Anpassung basierend auf der Position
        let position_add = if let Some(tags) = &self.hydrant.tags {
            match tags.position {
                Some(HydrantPosition::Lane) => 0.0,
                Some(HydrantPosition::Sidewalk) => 0.0,
                Some(HydrantPosition::ParkingLot) => 30.0, // Schlechter (Zufahrt/Zugriff)
                Some(HydrantPosition::Green) => 10.0,     // Etwas schlechter (Gelände)
                None => 5.0,
            }
        } else {
            5.0
        };

        // Score-Anpassung basierend auf der Anzahl der Kupplungen
        let couplings_add = if let Some(tags) = &self.hydrant.tags {
            if let Some(coup_str) = &tags.couplings {
                let coup_f = coup_str.parse::<i8>().unwrap_or(1);
                if coup_f >= 2 {
                    -30.0 // Besser
                } else {
                    0.0
                }
            } else {
                0.0
            }
        } else {
            0.0
        };

        // Hauptformel: Distanz (quadratisch gewichtet) + Durchmesser-Einfluss + Typ/Position/Kupplungen
        ((self.route_details.distance_meters / 100.0).powi(2) * 50.0)
            + ((150.0 / diameter) * 100.0)
            + type_add
            + position_add
            + couplings_add
    }
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

impl HydrantTags {
    /// Gibt die bekannten Eigenschaften des Hydranten auf der Konsole aus.
    pub fn output(&self) {
        if let Some(shape) = &self.shape {
            println!("🧯 Es handelt sich um einen {}", shape.to_display_name());
        }
        if let Some(diameter) = &self.diameter {
            println!("🧯 Das Rohr hat einen Durchmesser von {}mm", diameter);
        }
        if let Some(position) = &self.position {
            match position {
                HydrantPosition::Green => println!("🧯 Der Hydrant befindet sich auf einer Grünfläche."),
                HydrantPosition::Lane => println!("🧯 Der Hydrant befindet sich auf der Straße."),
                HydrantPosition::ParkingLot => println!("🧯 Der Hydrant befindet sich auf einem Parkplatz."),
                HydrantPosition::Sidewalk => println!("🧯 Der Hydrant befindet sich auf dem Gehweg."),
            }
        }

        if let Some(couplings) = &self.couplings {
            println!("🧯 Der Hydrant hat {} Kupplung(en).", couplings);
        }
    }
}