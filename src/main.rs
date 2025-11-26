mod api;
mod models;
mod utils;

use crate::utils::get_user_coordinates;
use anyhow::Result;
use dotenvy::dotenv;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();

    // 1. Benutzerkoordinaten abrufen
    let user_location = get_user_coordinates().await?;
    println!(
        "📍 Standort gefunden: Breite={}, Länge={}.",
        user_location.latitude, user_location.longitude
    );

    // 2. Die 5 nächstgelegenen Hydranten (Luftlinie) finden
    let hydrants = api::get_nearest_hydrants(&user_location, Some(5)).await?;

    if hydrants.is_empty() {
        println!("🔍 Keine Hydranten in der Nähe gefunden. Das Programm wird beendet.");
        return Ok(());
    }

    // 3. Den nächstgelegenen Hydranten per Route (Fußweg) ermitteln
    let best_hydrant = api::get_closest_by_route(&user_location, &hydrants).await?;
    println!(
        "🔥 Nächster Hydrant gefunden: Distanz: {:.1}m.",
        best_hydrant.route_details.distance_meters
    );
    best_hydrant.hydrant.tags.clone().expect("REASON").output();

    // 4. Statische Karte erstellen und speichern
    let map_bytes = api::get_map(&user_location, &best_hydrant, 800, 600).await?;
    let file_path = "hydrant_route.png";
    let mut file = File::create(file_path).await?;
    file.write_all(&map_bytes).await?;

    // 5. Karte öffnen
    utils::show_image_in_minifb(&file_path)?;



    Ok(())
}
