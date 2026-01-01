use anyhow::Result;
use chrono::{Utc, NaiveDateTime, Duration};
use geohash::{encode, Coord};
use mysql::prelude::*;
use mysql::*;
use serde_json;

use crate::models::{Location, RankedHydrant, CachedRouteItem, HydrantTags, Hydrant, RouteDetails};

const URL: &str = "mysql://root@localhost:3306/hydranten_cache";
const FORMAT: &str = "%Y-%m-%d %H:%M:%S";

pub fn get_ranked_hydrants(location: &Location)-> Result<Vec<RankedHydrant>> {
    let c = Coord { x: location.longitude, y: location.latitude};
    let geohash = encode(c, 8usize)?;

    let pool = Pool::new(URL)?;
    let mut conn = pool.get_conn()?;
    let response: Vec<(String, String)> = conn.query(format!(
        "SELECT routes, created_at FROM route_cache WHERE geohash = '{}'",
        geohash
    ))?;

    if response.is_empty() {
        return Ok(Vec::new());
    }

    println!("{:?}", response);

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

        let item_response: Vec<(String, String, String)> = conn.query(format!(
            "SELECT lat, lon, tags FROM hydrants WHERE id = '{}'",
            route_item.hydrant_id
        ))?;

        let lat = item_response[0].0.parse::<f64>()?;
        let lon = item_response[0].1.parse::<f64>()?;
        let tags = &item_response[0].2;
        let hyd_tags:HydrantTags  = serde_json::from_str(tags)?;


        let hydrant = Hydrant{
            longitude: lon,
            latitude: lat,
            tags: Option::from(hyd_tags),
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