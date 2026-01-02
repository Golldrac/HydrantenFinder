use std::sync::OnceLock;

use anyhow::{anyhow, Context, Result};
use chrono::{Duration, NaiveDateTime, Utc};
use geohash::{encode, Coord};
use mysql::prelude::*;
use mysql::*;
use serde_json;

use crate::models::{CachedRouteItem, Hydrant, HydrantTags, Location, RankedHydrant, RouteDetails};

/// Die Verbindungs-URL zur lokalen MySQL-Datenbank.
const URL: &str = "mysql://root@localhost:3306/hydranten_cache";

/// Globaler Datenbank-Pool, der einmalig beim Start initialisiert wird.
static DB_POOL: OnceLock<Pool> = OnceLock::new();

/// Initialisiert den Datenbank-Pool und erstellt die notwendigen Tabellenstrukturen,
/// falls diese noch nicht existieren.
///
/// # Errors
/// Gibt einen Fehler zurück, wenn die Verbindung fehlschlägt oder Tabellen nicht erstellt werden können.
pub fn initialize_db() -> Result<()> {
    let pool = Pool::new(URL).context("Verbindung zum Datenbank-Server fehlgeschlagen")?;
    let mut conn = pool.get_conn().context("Fehler beim Abrufen einer Verbindung aus dem Pool")?;

    // Hydranten-Stammdaten: Speichert Position und Metadaten (OSM-Tags)
    conn.query_drop(
        r"CREATE TABLE IF NOT EXISTS hydrants (
            id INT AUTO_INCREMENT PRIMARY KEY,
            osm_id BIGINT NOT NULL UNIQUE,
            lat DECIMAL(10, 8) NOT NULL,
            lon DECIMAL(11, 8) NOT NULL,
            tags JSON,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP
        ) ENGINE=InnoDB CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;",
    )?;

    // Cache für Routenberechnungen: Verknüpft einen Standort (Geohash) mit Hydranten-IDs
    conn.query_drop(
        r"CREATE TABLE IF NOT EXISTS route_cache (
            geohash VARCHAR(8) NOT NULL PRIMARY KEY,
            routes JSON NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        ) ENGINE=InnoDB CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;",
    )?;

    DB_POOL
        .set(pool)
        .map_err(|_| anyhow!("Datenbank-Pool wurde bereits initialisiert"))?;

    Ok(())
}

/// Gibt eine Referenz auf den globalen Datenbank-Pool zurück.
///
/// # Panics
/// Panikt, wenn der Pool vor dem Zugriff nicht mittels `initialize_db` initialisiert wurde.
pub fn get_pool() -> &'static Pool {
    DB_POOL.get().expect("Datenbank-Pool wurde nicht initialisiert!")
}

/// Ruft bereits berechnete Hydranten-Rankings für einen bestimmten Standort aus dem Cache ab.
///
/// Berücksichtigt nur Einträge, die nicht älter als 2 Wochen sind.
///
/// # Errors
/// Liefert Fehler bei Datenbankproblemen oder Deserialisierungsfehlern von JSON-Daten.
pub fn get_ranked_hydrants(location: &Location) -> Result<Vec<RankedHydrant>> {
    let coord = Coord {
        x: location.longitude,
        y: location.latitude,
    };
    let geohash = encode(coord, 8usize)?;

    let mut conn = get_pool().get_conn()?;

    // Abfrage des Caches für den spezifischen Geohash
    let row: Option<(String, NaiveDateTime)> = conn.exec_first(
        "SELECT routes, created_at FROM route_cache WHERE geohash = :gh",
        params! { "gh" => &geohash },
    )?;

    let (cached_routes_json, created_at) = match row {
        Some(data) => data,
        None => return Ok(Vec::new()),
    };

    // Validierung des Alters (maximal 2 Wochen)
    let now = Utc::now().naive_utc();
    if now.signed_duration_since(created_at) > Duration::weeks(2) {
        return Ok(Vec::new());
    }

    let cached_items: Vec<CachedRouteItem> = serde_json::from_str(&cached_routes_json)?;
    let mut ranked_hydrants = Vec::with_capacity(cached_items.is_empty().then_some(0).unwrap_or(cached_items.len()));

    for item in cached_items {
        // Details zum Hydranten aus der Stammdaten-Tabelle laden
        let hydrant_row: Option<(u64, f64, f64, String)> = conn.exec_first(
            "SELECT osm_id, lat, lon, tags FROM hydrants WHERE id = :id",
            params! { "id" => item.hydrant_id },
        )?;

        if let Some((osm_id, lat, lon, tags_str)) = hydrant_row {
            let tags: HydrantTags = serde_json::from_str(&tags_str)?;

            let hydrant = Hydrant {
                osm_id,
                longitude: lon,
                latitude: lat,
                tags: Some(tags),
            };

            let details = RouteDetails {
                distance_meters: item.distance,
                geometry_polyline: item.polyline,
            };

            ranked_hydrants.push(RankedHydrant::new(hydrant, details));
        }
    }

    Ok(ranked_hydrants)
}

/// Speichert eine Liste von gerankten Hydranten im Cache und aktualisiert die Stammdaten.
///
/// Nutzt `ON DUPLICATE KEY UPDATE`, um bestehende Hydranten-Informationen zu aktualisieren.
///
/// # Errors
/// Gibt einen Fehler zurück, wenn die Serialisierung fehlschlägt oder die Datenbank nicht erreichbar ist.
pub fn save_ranked_hydrants(location: &Location, hydrants: &[RankedHydrant]) -> Result<()> {
    let mut conn = get_pool().get_conn()?;
    let mut cached_route_items = Vec::with_capacity(hydrants.len());

    for ranked in hydrants {
        let h = &ranked.hydrant;
        let tags_json = serde_json::to_string(&h.tags)?;

        // Hydrant in Stammdaten sichern/aktualisieren
        conn.exec_drop(
            r"INSERT INTO hydrants (osm_id, lat, lon, tags)
              VALUES (:osm_id, :lat, :lon, :tags)
              ON DUPLICATE KEY UPDATE
              lat = VALUES(lat), lon = VALUES(lon), tags = VALUES(tags)",
            params! {
                "osm_id" => h.osm_id,
                "lat" => h.latitude,
                "lon" => h.longitude,
                "tags" => tags_json,
            },
        )?;

        // Interne ID für die Relation im route_cache abrufen
        let hydrant_id: i32 = conn.exec_first(
            "SELECT id FROM hydrants WHERE osm_id = :osm_id",
            params! { "osm_id" => h.osm_id },
        )?
            .context("Fehler beim Abrufen der Hydranten-ID nach dem Insert")?;

        cached_route_items.push(CachedRouteItem {
            hydrant_id,
            distance: ranked.route_details.distance_meters,
            polyline: ranked.route_details.geometry_polyline.clone(),
        });
    }

    // Routen-Cache für den aktuellen Standort speichern
    let coord = Coord {
        x: location.longitude,
        y: location.latitude,
    };
    let geohash = encode(coord, 8usize)?;

    conn.exec_drop(
        r"INSERT INTO route_cache (geohash, routes, created_at)
          VALUES (:geohash, :routes, NOW())
          ON DUPLICATE KEY UPDATE
          routes = VALUES(routes), created_at = NOW()",
        params! {
            "geohash" => geohash,
            "routes" => serde_json::to_string(&cached_route_items)?,
        },
    )?;

    Ok(())
}