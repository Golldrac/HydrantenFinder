use crate::models::{Hydrant, Location};
use anyhow::Result;
use std::process::exit;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use dotenvy::dotenv;
use geo::{Distance, Haversine};
use geo_types::Point;
use reqwest::Client;
use windows::Devices::Geolocation::{
    Geocoordinate, GeolocationAccessStatus, Geolocator, Geoposition, PositionAccuracy,
};
use windows::Foundation::TimeSpan;
use windows_future::IAsyncOperation;

/// Konvertiert Sekunden in die 100-Nanosekunden-Einheit, die für `TimeSpan::Duration` benötigt wird.
const DURATION_FACTOR: i64 = 10_000_000; // 1 Sekunde = 10 Millionen * 100ns

/// Die maximale Dauer, für die eine gespeicherte Position akzeptiert wird (5 Minuten).
const MAX_AGE_DURATION_NANOS: i64 = 5 * 60 * DURATION_FACTOR;

/// Der Timeout für die Positionsanforderung (5 Sekunden).
const TIMEOUT_DURATION_NANOS: i64 = 5 * DURATION_FACTOR;
/// Der maximale Abstand in Metern, bei dem ein Hydrant als "erreicht" gilt und keine Routenberechnung nötig ist.
pub const PROXIMITY_THRESHOLD_METERS: f64 = 5.0;
/// Die Breite der generierten statischen Karte in Pixeln (für Zoom-Berechnung).
const MAP_WIDTH_PIXELS: f64 = 800.0;
/// Der maximale Zoomlevel für die Stadia Maps API.
const MAX_ZOOM: u8 = 32;

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
            anyhow::bail!("[X] Standortzugriff wurde in den Windows-Einstellungen verweigert.");
        }
        GeolocationAccessStatus::Unspecified => {
            anyhow::bail!("[!] Unspezifizierter Fehler beim Abrufen des Standortzugriffs.");
        }
        _ => anyhow::bail!("[?] Unbekannter Standortzugriffsstatus."),
    }
}

/// Berechnet einen geeigneten Zoomlevel für die statische Karte,
/// sodass beide Standorte auf die Karte passen.
///
/// Nutzt eine Näherungsformel, die auf der horizontalen Distanz zwischen den Punkten basiert.
///
/// # Arguments
///
/// * `loc1` - Die erste Position (z.B. Benutzer).
/// * `loc2` - Die zweite Position (z.B. Hydrant).
///
/// # Returns
///
/// Der berechnete Zoomlevel als `u32`, begrenzt auf $0$ bis $18$.
pub fn calculate_zoom_level(loc1: &Location, loc2: &Hydrant) -> u8 {

    let p1 = Point::new(loc1.longitude, loc1.latitude);
    let p2 = Point::new(loc2.longitude, loc2.latitude);
    let distance_m = Haversine.distance(p1, p2);

    if distance_m <= PROXIMITY_THRESHOLD_METERS {
        return MAX_ZOOM;
    }

    let fraction_of_map = 0.4;
    let meters_per_pixel = distance_m / (MAP_WIDTH_PIXELS * fraction_of_map);

    let mid_lat_rad = (loc1.latitude.to_radians() + loc2.latitude.to_radians()) / 2.0;
    let earth_circumference = 40_075_016.7;
    let zoom_f64 = (earth_circumference * mid_lat_rad.cos() / (256.0 * meters_per_pixel)).log2() * 1.1;

    let final_zoom = zoom_f64.floor() as u8;

    final_zoom.clamp(0, MAX_ZOOM)
}

/// Initialisiert die Anwendung (Umgebungsvariablen und Terminal-Einstellungen)
pub fn init_app()-> Result<(Client, Arc<AtomicBool>, Arc<AtomicBool>)> {
    // 1. .env laden (Plattformunabhängig)
    dotenv().ok();
    setup_terminal();
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
            println!("[+] Datenbank aktiv. Beende nach Abschluss...");
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
        use windows_sys::Win32::System::Console::{
            GetStdHandle, GetConsoleMode, SetConsoleMode,
            SetConsoleOutputCP, SetConsoleCP,
            ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_OUTPUT_HANDLE
        };

        SetConsoleOutputCP(65001);
        SetConsoleCP(65001);

        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        if GetConsoleMode(handle, &mut mode) != 0 {
            let _ = SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
        }
    }
}

#[cfg(not(windows))]
fn setup_terminal() {
    // Auf Linux/macOS ist in der Regel nichts zu tun,
    // da UTF-8 und ANSI standardmäßig aktiv sind.
}