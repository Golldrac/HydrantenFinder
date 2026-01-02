use anyhow::Result;
use chrono::{Utc, NaiveDateTime, Duration};
use geohash::{encode, Coord};
use mysql::prelude::*;
use mysql::*;
use serde_json;
use std::sync::OnceLock;

use crate::models::{Location, RankedHydrant, CachedRouteItem, HydrantTags, Hydrant, RouteDetails};

const URL: &str = "mysql://root@localhost:3306/hydranten_cache";
const FORMAT: &str = "%Y-%m-%d %H:%M:%S";

static DB_POOL: OnceLock<Pool> = OnceLock::new();


/// Erstellt die Tabellen, falls sie fehlen, und prüft die Verbindung.
pub fn initialize_db() -> Result<()> {
    let pool = Pool::new(URL)?;
    let mut conn = pool.get_conn()?;

    // 1. Hydranten Tabelle
    conn.query_drop(
        r"CREATE TABLE IF NOT EXISTS hydrants (
            id INT AUTO_INCREMENT PRIMARY KEY,
            osm_id BIGINT NOT NULL UNIQUE,
            lat DECIMAL(10, 8) NOT NULL,
            lon DECIMAL(11, 8) NOT NULL,
            tags JSON,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP
        ) ENGINE=InnoDB CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;"
    )?;

    // 2. Route Cache Tabelle
    conn.query_drop(
        r"CREATE TABLE IF NOT EXISTS route_cache (
            geohash VARCHAR(8) NOT NULL PRIMARY KEY,
            routes JSON NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        ) ENGINE=InnoDB CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;"
    )?;

    // Pool im globalen Speicher ablegen
    DB_POOL.set(pool).map_err(|_| anyhow::anyhow!("Fehler beim Initialisieren des DB-Pools"))?;

    Ok(())
}

pub fn get_pool() -> &'static Pool {
    DB_POOL.get().expect("Datenbank-Pool wurde nicht initialisiert!")
}

pub fn get_ranked_hydrants(location: &Location)-> Result<Vec<RankedHydrant>> {
    let c = Coord { x: location.longitude, y: location.latitude};
    let geohash = encode(c, 8usize)?;

    let mut conn = get_pool().get_conn()?;
    let response: Vec<(String, String)> = conn.query(format!(
        "SELECT routes, created_at FROM route_cache WHERE geohash = '{}'",
        geohash
    ))?;

    if response.is_empty() {
        return Ok(Vec::new());
    }

    let created_at = &response[0].1;

    if let Ok(naive_dt) = NaiveDateTime::parse_from_str(created_at, FORMAT) {

        let past_time = naive_dt.and_utc();
        let now = Utc::now();

        let duration = now.signed_duration_since(past_time);

        if duration > Duration::weeks(2) {
            return Ok(Vec::new());
        }

    }

    //TODO: Errorhandling verbessern
    let cached_routes = &response[0].0;
    let cached_routes_json: Vec<CachedRouteItem> = serde_json::from_str(cached_routes)?;

    let mut ret: Vec<RankedHydrant> = Vec::new();

    for route_item in cached_routes_json{

        let item_response: Vec<(u64, f64, f64, String)> = conn.query(format!(
            "SELECT osm_id, lat, lon, tags FROM hydrants WHERE id = '{}'",
            route_item.hydrant_id
        ))?;

        if item_response.is_empty() {
            continue; // Falls ein Hydrant fehlt, überspringen
        }

        let (osm_id, lat, lon, tags_json) = &item_response[0];
        let hyd_tags: HydrantTags = serde_json::from_str(tags_json)?;


        let hydrant = Hydrant {
            osm_id: *osm_id,
            longitude: *lon,
            latitude: *lat,
            tags: Some(hyd_tags),
        };

        let details = RouteDetails{
            distance_meters: route_item.distance,
            geometry_polyline: route_item.polyline,
        };

        let ranked_hydrant = RankedHydrant::new(hydrant, details);

        ret.push(ranked_hydrant);

    }

    Ok(ret)
}

pub fn save_ranked_hydrants(location: &Location, hydrants: &Vec<RankedHydrant>)-> Result<()>{

    let mut cached_route_items = Vec::new();

    let mut conn = get_pool().get_conn()?;

    for hydrant in hydrants {
        let osm_id = hydrant.hydrant.osm_id;
        let lat = hydrant.hydrant.latitude;
        let lon = hydrant.hydrant.longitude;
        let tags = serde_json::to_string(&hydrant.hydrant.tags)?;

        conn.exec_drop(
            r"INSERT INTO hydrants (osm_id, lat, lon, tags)
                    VALUES (:osm_id, :lat, :lon, :tags)
                    ON DUPLICATE KEY UPDATE
                    lat = VALUES(lat),
                    lon = VALUES(lon),
                    tags = VALUES(tags)",
            params! {
                    "osm_id" => osm_id,
                    "lat" => lat,
                    "lon" => lon,
                    "tags" => tags, // tags ist hier der JSON-String
            },
        )?;

        let response: Vec<String> = conn.query(format!(
            "SELECT id FROM hydrants WHERE osm_id = {};", osm_id
        ))?;

        let hydrant_id = response[0].parse::<i32>()?;

        let route_item = CachedRouteItem {
            hydrant_id,
            distance: hydrant.route_details.distance_meters,
            polyline: hydrant.route_details.geometry_polyline.parse().unwrap(),
        };

        cached_route_items.push(route_item);

    }

    let c = Coord { x: location.longitude, y: location.latitude};
    let geohash = encode(c, 8usize)?;

    conn.exec_drop(
        r"INSERT INTO route_cache (geohash, routes, created_at)
                VALUES (:geohash, :routes, NOW())
                ON DUPLICATE KEY UPDATE
                routes = VALUES(routes),
                created_at = NOW()",
        params! {
                "geohash" => geohash,
                "routes" => serde_json::to_string(&cached_route_items)?,
        },
    )?;
    Ok(())
}