mod api;
mod models;
mod utils;
mod database;

use std::sync::Arc;

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

    let (client, running, in_db) = init_app()?;

    // 1. Benutzerkoordinaten abrufen

    let user_location = utils::get_user_coordinates().await?;
    println!(
        "[+] Standort gefunden: Breite={}, Länge={}.",
        user_location.latitude,
        user_location.longitude
    );

    let mut sorted_hydrants = database::get_cached_hydrant_routes(&user_location)
        .unwrap_or_else(|e| {
            eprintln!("[!] Cache-Fehler: {}. Fahre mit API-Abfrage fort...", e);
            Vec::new()
        });
    let mut db_handle= None;
    if sorted_hydrants.is_empty(){
        // 2. Die 5 nächstgelegenen Hydranten (Luftlinie) finden
        let hydrants = api::get_nearest_hydrants(&user_location, Some(5), &client).await?;

        if hydrants.is_empty() {
            println!("[-] Keine Hydranten in der Nähe gefunden. Das Programm wird beendet.");
            return Ok(());
        }

        // 3. Den am besten bewerteten Hydranten per Route (Fußweg) ermitteln und ranken
        sorted_hydrants = api::get_closest_by_route(&user_location, &hydrants, &client).await?;
        let list = sorted_hydrants.clone();
        let loc = user_location.clone();
        let r_for_db = Arc::clone(&running);
        let db_for_db = Arc::clone(&in_db);

        db_handle = Some(tokio::task::spawn_blocking(move || {
            if let Err(e) = database::save_hydrant_routes(&loc, &list, &r_for_db, &db_for_db) {
                eprintln!("[!] Fehler beim Speichern im Cache: {}", e);
            }
        }));


    }

    let best_hydrant = &sorted_hydrants[0];

    // Ausgabe der besten Route
    println!(
        "[+] Nächster Hydrant gefunden: Distanz: {:.1}m.",
        best_hydrant.route_details.distance_meters
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
    tokio::task::spawn_blocking(move || utils::show_image_in_minifb(file_path)).await??;
    if let Some(handle) = db_handle {
        let _ = handle.await;
    }

    Ok(())
}