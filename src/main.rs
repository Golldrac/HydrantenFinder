mod api;
mod database;
mod gui;
mod models;
mod utils;

use std::default::Default;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use eframe::egui;

use crate::gui::AppState;
use crate::utils::init_app;

//TODO: Retrylogik
/// Einstiegspunkt der Anwendung.
///
/// Initialisiert den geteilten [`AppState`], startet [`run_search`] in einem Tokio-Hintergrundtask
/// und öffnet anschließend das maximierte GUI-Fenster auf dem Main-Thread.
///
/// # Returns
///
/// `Ok(())`, sobald das GUI-Fenster regulär geschlossen wurde.
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn `eframe::run_native` beim Starten oder Ausführen der GUI fehlschlägt.
///
/// # Panics
///
/// Panikt, wenn der `Mutex` von `state` bei der Fehlerbehandlung im Hintergrundtask vergiftet ist.
#[tokio::main]
async fn main() -> Result<()> {
    let state = Arc::new(Mutex::new(AppState::default()));

    // Hintergrund-Task für Standort- und Hydrantensuche starten
    let value = state.clone();
    tokio::spawn(async move {
        let mut value = value;
        if let Err(e) = run_search(&mut value).await {
            eprintln!("[!] Fehler: {e:#}");
            let mut s = value.lock().unwrap();
            s.set_error_banner(true);
            s.set_loading(false);
            s.set_error_msg(format!("Fehler aufgetreten:\n{e}"));
        }
    });

    // GUI auf dem Main-Thread starten (blockiert, bis das Fenster geschlossen wird)
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_maximized(true),
        ..Default::default()
    };
    let gui_status = Arc::clone(&state);
    eframe::run_native(
        "Hydrantenfinder",
        options,
        Box::new(move |cc| Ok(Box::new(gui::MyEguiApp::new(cc, gui_status)))),
    )
        .map_err(|e| anyhow::anyhow!("GUI-Fehler: {e}"))?;

    Ok(())
}

/// Führt den gesamten Such- und Caching-Ablauf aus und aktualisiert den [`AppState`].
///
/// 1. Initialisiert Anwendung und Datenbank über [`init_app`].
/// 2. Ermittelt den Benutzerstandort über [`utils::get_user_coordinates`].
/// 3. Prüft den Datenbank-Cache über [`database::get_cached_hydrant_routes`].
/// 4. Fragt bei einem Cache-Miss die 5 nächsten Hydranten ab, berechnet deren Fußweg-Routen
///    und speichert die Ergebnisse asynchron im Cache.
///
/// # Arguments
///
/// * `state` - Veränderliche Referenz auf den geteilten [`AppState`].
///
/// # Returns
///
/// `Ok(())` nach Abschluss der Suche und eventueller Datenbank-Speicherung.
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn die Initialisierung, Standortabfrage, Hydrantensuche
/// oder Routenberechnung fehlschlägt.
///
/// # Panics
///
/// Panikt, wenn der `Mutex` von `state` vergiftet (poisoned) ist.
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

    {
        let mut s = state.lock().unwrap();
        s.set_hydrants(sorted_hydrants.clone());
        s.set_loading(false);
    }

    if let Some(handle) = db_handle {
        let _ = handle.await;
    }

    Ok(())
}