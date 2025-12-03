mod api;
mod models;
mod utils;

use anyhow::Result;
use dotenvy::dotenv;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

/// Hauptfunktion der Anwendung. Führt den gesamten Ablauf aus:
/// 1. Laden der Umgebungsvariablen.
/// 2. Abrufen des aktuellen Benutzerstandorts.
/// 3. Suchen der nächstgelegenen Hydranten (Luftlinie).
/// 4. Ermitteln und Ranken des schnellsten Hydranten per Route (Fußweg).
/// 5. Generieren und Speichern einer statischen Karte der Route.
/// 6. Anzeigen der Karte in einem Fenster.
#[tokio::main]
async fn main() -> Result<()> {
    // Umgebungsvariablen aus .env laden
    dotenv().ok();

    // 1. Benutzerkoordinaten abrufen
    let user_location = utils::get_user_coordinates().await?;
    println!(
        "📍 Standort gefunden: Breite={}, Länge={}.",
        user_location.latitude, user_location.longitude
    );

    // 2. Die 5 nächstgelegenen Hydranten (Luftlinie) finden
    // Der Radius (500m) ist in api::get_nearest_hydrants fest codiert.
    let hydrants = api::get_nearest_hydrants(&user_location, Some(5)).await?;

    if hydrants.is_empty() {
        println!("🔍 Keine Hydranten in der Nähe gefunden. Das Programm wird beendet.");
        return Ok(());
    }

    // 3. Den am besten bewerteten Hydranten per Route (Fußweg) ermitteln und ranken
    let ranked_hydrants = api::get_closest_by_route(&user_location, &hydrants).await?;
    let best_hydrant = &ranked_hydrants[0];

    // Ausgabe der besten Route
    println!(
        "🔥 Nächster Hydrant gefunden: Distanz: {:.1}m. Score: {:.2}",
        best_hydrant.route_details.distance_meters,
        best_hydrant.score
    );

    // Ausgabe der Hydranten-Tags, falls vorhanden
    if let Some(tags) = best_hydrant.hydrant.tags.clone() {
        tags.output();
    }

    // 4. Statische Karte erstellen und speichern
    let map_bytes = api::get_map(&user_location, &best_hydrant, 800, 600).await?;
    let file_path = "hydrant_route.png";
    let mut file = File::create(file_path).await?;
    file.write_all(&map_bytes).await?;

    // 5. Karte öffnen
    utils::show_image_in_minifb(&file_path)?;

    Ok(())
}