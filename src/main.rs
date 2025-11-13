mod api;
mod models;
mod utils;

use tokio;
use dotenvy::dotenv;
use anyhow::Result;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use opener;

use crate::utils::get_user_coordinates;


const POLYLINE_PRECISION: u32 = 6;


#[tokio::main]
async fn main() -> Result<()> {
    //let start_lat = 49.79; // Beispiel: Würzburg
    //let start_lon = 9.93;
    //let user_location = Coordinate::new(start_lon, start_lat);
    dotenv().ok();
    let user_location = get_user_coordinates().await?;

    let hydrants = api::get_nearest_hydrants(&user_location, Some(5)).await?;
    if hydrants.is_empty() {
        println!("Keine Hydranten in der Nähe gefunden. Das Programm wird beendet.");
        return Ok(());
    }

    let best_hydrant = api::get_closest_by_route(&user_location, &hydrants).await?;

    let map_bytes = api::get_map_post(&user_location, &best_hydrant, 800, 600).await?;
    let file_path = "hydrant_route.png";
    let mut file = File::create(file_path).await?;
    file.write_all(&map_bytes).await?;
    opener::open(file_path)?;


    Ok(())
}