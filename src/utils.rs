use crate::models::Location;
use anyhow::Result;
use image::GenericImageView;
use indicatif::{ProgressBar, ProgressStyle};
use minifb::{Key, Window, WindowOptions};
use std::path::Path;
use std::process::exit;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use dotenvy::dotenv;
use reqwest::Client;
use windows::Devices::Geolocation::{
    Geocoordinate, GeolocationAccessStatus, Geolocator, Geoposition, PositionAccuracy,
};
use windows::Foundation::TimeSpan;
use windows_future::IAsyncOperation;
#[cfg(windows)]
use windows_sys::Win32::System::Console::{
    GetStdHandle, GetConsoleMode, SetConsoleMode,
    ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_OUTPUT_HANDLE
};

/// Konvertiert Sekunden in die 100-Nanosekunden-Einheit, die für `TimeSpan::Duration` benötigt wird.
const DURATION_FACTOR: i64 = 10_000_000; // 1 Sekunde = 10 Millionen * 100ns

/// Die maximale Dauer, für die eine gespeicherte Position akzeptiert wird (5 Minuten).
const MAX_AGE_DURATION_NANOS: i64 = 5 * 60 * DURATION_FACTOR;

/// Der Timeout für die Positionsanforderung (5 Sekunden).
const TIMEOUT_DURATION_NANOS: i64 = 5 * DURATION_FACTOR;

/// Der Skalierungsfaktor, der auf das geladene Bild angewendet wird.
///
/// Ein Wert unter 1.0 bewirkt ein Herauszoomen (Bild wird verkleinert, um auf den Bildschirm zu passen),
/// während ein Wert über 1.0 ein Hineinzoomen bewirkt.
const SCALE_FACTOR: f32 = 0.6;

/// Ruft die aktuellen geographischen Koordinaten des Benutzers über die Windows Geolocation API ab.
///
/// Stellt sicher, dass der Standortzugriff gewährt ist und versucht,
/// eine Position mit einem maximalen Alter von 5 Minuten und einem Timeout von 5 Sekunden zu erhalten.
///
/// # Errors
/// Gibt einen Fehler zurück, wenn:
/// * Der Standortzugriff in den Windows-Einstellungen verweigert wurde.
/// * Die Windows API einen Fehler zurückgibt (z.B. Timeout oder interner Fehler).

pub async fn get_user_coordinates() -> Result<Location> {
    // ... (Funktionsimplementierung unverändert)
    let access_status = Geolocator::RequestAccessAsync()?.await?;

    match access_status {
        GeolocationAccessStatus::Allowed => {
            let geolocator = Geolocator::new()?;
            geolocator.SetDesiredAccuracy(PositionAccuracy::High)?;

            let max_age = TimeSpan {
                Duration: MAX_AGE_DURATION_NANOS,
            };
            let timeout = TimeSpan {
                Duration: TIMEOUT_DURATION_NANOS,
            };

            let operation: IAsyncOperation<Geoposition> =
                geolocator.GetGeopositionAsyncWithAgeAndTimeout(max_age, timeout)?;

            let geoposition = operation.await?;
            let geocoordinate: Geocoordinate = geoposition.Coordinate()?;

            let loc = Location::new(geocoordinate.Latitude()?, geocoordinate.Longitude()?);

            Ok(loc)
        }
        GeolocationAccessStatus::Denied => {
            anyhow::bail!("❌ Standortzugriff wurde in den Windows-Einstellungen verweigert.");
        }
        GeolocationAccessStatus::Unspecified => {
            anyhow::bail!("⚠️ Unspezifizierter Fehler beim Abrufen des Standortzugriffs.");
        }
        // Deckt zukünftige/unbekannte Status ab.
        _ => anyhow::bail!("❓ Unbekannter Standortzugriffsstatus."),
    }
}

/// Lädt ein Bild von einem Pfad, skaliert es (entsprechend `SCALE_FACTOR`)
/// und zeigt es in einem `minifb`-Fenster an.
///
/// Während der Lade- und Konvertierungsphase wird ein Konsolen-Spinner angezeigt.
///
/// # Arguments
///
/// * `file_path` - Der Pfad zur Bilddatei (z.B. "hydrant_route.png").
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn das Bild nicht geladen/dekodiert werden kann oder
/// `minifb`-Operationen fehlschlagen.
pub fn show_image_in_minifb(file_path: &str) -> Result<()> {
    // ... (Funktionsimplementierung unverändert)
    // 1. Spinner initialisieren
    let spinner_style = ProgressStyle::with_template("{spinner:.cyan.bold} {msg}")?;
    let pb = ProgressBar::new_spinner();
    pb.set_style(spinner_style);
    pb.set_message("🖼️ Lade und berechne Bildpuffer...  .");
    pb.enable_steady_tick(std::time::Duration::from_millis(100));

    // 2. Das Bild laden, dekodieren und skalieren
    let img = image::open(Path::new(file_path))?;

    // Berechne die neuen Abmessungen basierend auf dem globalen Skalierungsfaktor
    let new_width = (img.width() as f32 * SCALE_FACTOR) as u32;
    let new_height = (img.height() as f32 * SCALE_FACTOR) as u32;

    // Skaliere das Bild auf die neue Größe (Herauszoomen).
    let resized_img =
        img.resize_exact(new_width, new_height, image::imageops::FilterType::Nearest);

    // Abmessungen des *skalierten* Bildes abrufen
    let width = resized_img.width() as usize;
    let height = resized_img.height() as usize;

    // 3. Den Pixelpuffer vorbereiten
    let mut buffer: Vec<u32> = vec![0; width * height];

    // Iteriere über die Pixel des skalierten Bildes und konvertiere sie in das u32-Format.
    for (x, y, pixel) in resized_img.pixels() {
        // 'pixel.0' ist ein Array von [r, g, b, a] oder [r, g, b]
        let (r, g, b, a) = match pixel.0.len() {
            4 => (
                pixel.0[0] as u32,
                pixel.0[1] as u32,
                pixel.0[2] as u32,
                pixel.0[3] as u32,
            ),
            3 => (
                pixel.0[0] as u32,
                pixel.0[1] as u32,
                pixel.0[2] as u32,
                255u32,
            ),
            _ => continue,
        };

        // ARGB-Format: (Alpha << 24) | (Rot << 16) | (Grün << 8) | (Blau)
        let color = (a << 24) | (r << 16) | (g << 8) | b;

        // Berechne den Index im flachen Puffer
        let index = (y as usize) * width + (x as usize);
        buffer[index] = color;
    }

    pb.finish();
    println!("✅ Route gespeichert und Karte wird geöffnet: {}.", file_path);

    // 5. Das Fenster erstellen
    let mut window = Window::new(
        "Route (ESC zum Schließen)",
        width,
        height,
        // Optionen für das Fenster
        WindowOptions {
            scale: minifb::Scale::FitScreen,
            ..WindowOptions::default()
        },
    )?;

    // Begrenze die Aktualisierungsrate des Fensters auf 60 FPS
    window.limit_update_rate(Some(std::time::Duration::from_millis(1000 / 60)));


    // 6. Die Ereignisschleife (Loop)
    while window.is_open() && !window.is_key_down(Key::Escape) {
        // Aktualisiere den Fensterinhalt mit dem Pixelpuffer
        window.update_with_buffer(&buffer, width, height)?;
    }

    Ok(())
}

/// Initialisiert die Anwendung (Umgebungsvariablen und Terminal-Einstellungen)
pub fn init_app()-> Result<(Client, Arc<AtomicBool>, Arc<AtomicBool>)> {
    // 1. .env laden (Plattformunabhängig)
    dotenv().ok();
    // 2. Terminal konfigurieren
    setup_terminal();

    // 3. Datenbank initialisieren (Tabellen erstellen & Pool bereitstellen)
    crate::database::initialize_db()?;

    let client = reqwest::Client::builder()
        .user_agent("HydrantenFinder/1.0")
        .timeout(Duration::from_secs(30))
        .build()?;

    let running = Arc::new(AtomicBool::new(true));
    let in_db = Arc::new(AtomicBool::new(false));

    let r_sig = Arc::clone(&running);
    let db_sig = Arc::clone(&in_db);


    ctrlc::set_handler(move || {
        if db_sig.load(Ordering::SeqCst) {
            println!("🗄️ Datenbank aktiv. Beende nach Abschluss...");
            r_sig.store(false, Ordering::SeqCst);
        } else {
            exit(0);
        }
    }).expect("Error setting handler");

    Ok((client, running, in_db))
}

#[cfg(windows)]
fn setup_terminal() {
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        if GetConsoleMode(handle, &mut mode) != 0 {
            // Aktiviert ANSI-Sequenzen für Farben und Emojis
            let _ = SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
        }
        // Setzt die Codepage auf UTF-8
        let _ = std::process::Command::new("cmd")
            .args(&["/c", "chcp 65001"])
            .output();
    }
}

#[cfg(not(windows))]
fn setup_terminal() {
    // Auf Linux/macOS ist in der Regel nichts zu tun,
    // da UTF-8 und ANSI standardmäßig aktiv sind.
}