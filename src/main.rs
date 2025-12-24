mod api;
mod models;
mod utils;

use std::time::Duration;

use anyhow::Result;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use crate::utils::init_app;

/// Hauptfunktion der Anwendung. Führt den gesamten Ablauf aus:
/// 1. Initialisierung.
/// 2. Abrufen des aktuellen Benutzerstandorts.
/// 3. Suchen der nächstgelegenen Hydranten (Luftlinie).
/// 4. Ermitteln und Ranken des schnellsten Hydranten per Route (Fußweg).
/// 5. Generieren und Speichern einer statischen Karte der Route.
/// 6. Anzeigen der Karte in einem Fenster.
#[tokio::main]
async fn main() -> Result<()> {

    let _ = utils::test_db();
    /*
    init_app();

    let client = reqwest::Client::builder()
        .user_agent("HydrantenFinder/1.0")
        .timeout(Duration::from_secs(30))
        .build()?;

    // 1. Benutzerkoordinaten abrufen

    let user_location = utils::get_user_coordinates().await?;
    println!(
        "📍 Standort gefunden: Breite={}, Länge={}.",
        user_location.latitude, user_location.longitude
    );

    // 2. Die 5 nächstgelegenen Hydranten (Luftlinie) finden
    // Der Radius (500m) ist in api::get_nearest_hydrants fest codiert.
    let hydrants = api::get_nearest_hydrants(&user_location, Some(5), &client).await?;

    if hydrants.is_empty() {
        println!("🔍 Keine Hydranten in der Nähe gefunden. Das Programm wird beendet.");
        return Ok(());
    }

    // 3. Den am besten bewerteten Hydranten per Route (Fußweg) ermitteln und ranken
    let ranked_hydrants = api::get_closest_by_route(&user_location, &hydrants, &client).await?;
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
    let file_path = "hydrant_route.png";
    let map_bytes = api::get_map(&client, &user_location, &best_hydrant, 800, 600).await?;
    let mut file = File::create(file_path).await?;
    file.write_all(&map_bytes).await?;

    // Starte die Anzeige in einer blockierenden Task
    tokio::task::spawn_blocking(move || {
        utils::show_image_in_minifb(file_path)
    }).await??;
    */

    Ok(())
}