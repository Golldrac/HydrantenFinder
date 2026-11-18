mod api;
mod models;
mod utils;
mod database;
mod gui;

use std::sync::{Arc, Mutex};
use anyhow::Result;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use crate::gui::AppState;
use crate::utils::init_app;

#[tokio::main]
async fn main() -> Result<()> {
    let mut state = Arc::new(Mutex::new(AppState::default()));

    // Hintergrund-Task für Standort- und Hydrantensuche starten
    let value = state.clone();
    tokio::spawn(async move {
        let mut value = value.clone();
        if let Err(e) = run_search(&mut value).await {
            eprintln!("[!] Fehler: {e:#}");
            let mut s = value.lock().unwrap();
            s.set_error_banner(true);
            s.set_loading(false);
            s.set_error_msg(format!("Fehler aufgetreten:\n{e}"));
        }
    });

    // GUI auf dem Main-Thread starten (blockiert, bis das Fenster geschlossen wird)
    let native_options = eframe::NativeOptions::default();
    let gui_status = Arc::clone(&state);
    eframe::run_native(
        "Hydrantenfinder",
        native_options,
        Box::new(move |cc| Ok(Box::new(gui::MyEguiApp::new(cc, gui_status)))),
    )
        .map_err(|e| anyhow::anyhow!("GUI-Fehler: {e}"))?;

    Ok(())
}

async fn run_search(state: &mut Arc<Mutex<AppState>>) -> Result<()> {
    let (client, running, in_db) = init_app()?;


    // 1. Benutzerkoordinaten abrufen
    let user_location = utils::get_user_coordinates().await?;
    {
        let mut s = state.lock().unwrap();
        s.set_user_loc(user_location);
    }

    println!(
        "[+] Standort gefunden: Breite={}, Länge={}.",
        user_location.latitude, user_location.longitude
    );

    let mut sorted_hydrants = database::get_cached_hydrant_routes(&user_location)
        .unwrap_or_else(|e| {
            eprintln!("[!] Cache-Fehler: {}. Fahre mit API-Abfrage fort...", e);
            Vec::new()
        });

    let mut db_handle = None;
    if sorted_hydrants.is_empty() {

        // 2. Die 5 nächstgelegenen Hydranten (Luftlinie) finden
        let hydrants = api::get_nearest_hydrants(&user_location, Some(5), &client).await?;

        if hydrants.is_empty() {
            {
                let mut s = state.lock().unwrap();
               s.set_error_banner(true);
                s.set_loading(false);
                s.set_error_msg("Suche fehlgeschlagen.".into());
            }
            println!("[-] Keine Hydranten in der Nähe gefunden.");
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
    {
        let mut s = state.lock().unwrap();
        s.set_hydrants(sorted_hydrants.clone());
    }

    println!(
        "[+] Nächster Hydrant gefunden: Distanz: {:.1}m.",
        best_hydrant.route_details.distance_meters
    );

    if let Some(tags) = best_hydrant.hydrant.tags.clone() {
        println!("{}", tags.output());
        {
            let mut s = state.lock().unwrap();
            s.set_loading(false);
        }

    }else {
        {
            let mut s = state.lock().unwrap();
            s.set_loading(false);
        }
    }

    // 4. Statische Karte erstellen und speichern
    let file_path = "hydrant_route.png";
    let map_bytes = api::get_map(&client, &user_location, best_hydrant, 800, 600).await?;
    let mut file = File::create(file_path).await?;
    file.write_all(&map_bytes).await?;

    // Starte die Anzeige in einer blockierenden Task
    tokio::task::spawn_blocking(move || utils::show_image_in_minifb(file_path)).await??;
    if let Some(handle) = db_handle {
        let _ = handle.await;
    }

    Ok(())
}