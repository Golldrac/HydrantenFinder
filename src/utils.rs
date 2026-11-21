use std::process::exit;
use std::sync::Arc;
use std::time::Duration;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use dotenvy::dotenv;
use geo::{Distance, Haversine};
use geo_types::Point;
use reqwest::Client;
use windows::Devices::Geolocation::{
    Geocoordinate, GeolocationAccessStatus, Geolocator, Geoposition, PositionAccuracy,
};
use windows::Foundation::TimeSpan;
use windows_future::IAsyncOperation;

use crate::models::{Hydrant, Location};

/// Umrechnungsfaktor von Sekunden in 100-Nanosekunden-Einheiten für `TimeSpan::Duration`.
const DURATION_FACTOR: i64 = 10_000_000;

/// Maximales Alter einer zwischengespeicherten Windows-Standortposition (5 Minuten).
const MAX_AGE_DURATION_NANOS: i64 = 5 * 60 * DURATION_FACTOR;

/// Timeout für die Windows-Standortabfrage (5 Sekunden).
const TIMEOUT_DURATION_NANOS: i64 = 5 * DURATION_FACTOR;

/// Maximaler Abstand in Metern, unterhalb dessen ein Hydrant als unmittelbar erreicht gilt.
pub const PROXIMITY_THRESHOLD_METERS: f64 = 5.0;

/// Referenzbreite der Kartenansicht in Pixeln für die Zoom-Berechnung.
const MAP_WIDTH_PIXELS: f64 = 800.0;

/// Maximal zulässiger Zoomlevel für die Kartendarstellung.
const MAX_ZOOM: u8 = 32;

/// Ruft die aktuellen geographischen Koordinaten des Benutzers über die Windows Geolocation API ab.
///
/// Fordert die Standortberechtigung an und versucht, eine Position mit hoher Genauigkeit,
/// einem maximalen Alter von 5 Minuten und einem Timeout von 5 Sekunden zu ermitteln.
///
/// # Returns
///
/// Die ermittelte Benutzerposition als [`Location`].
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn:
/// * Der Standortzugriff in den Windows-Einstellungen verweigert wurde oder unspezifiziert ist.
/// * Die Windows Geolocation API einen Fehler oder ein Timeout meldet.
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

/// Berechnet einen geeigneten Karten-Zoomlevel, sodass beide Positionen sichtbar sind.
///
/// Nutzt die Haversine-Distanz zwischen beiden Punkten sowie eine Mercator-Näherungsformel.
///
/// # Arguments
///
/// * `loc1` - Die erste Position (z. B. Benutzerstandort).
/// * `loc2` - Die zweite Position (z. B. Zielhydrant).
///
/// # Returns
///
/// Der berechnete Zoomlevel als `u8`, begrenzt auf den Bereich `0` bis [`MAX_ZOOM`].
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

    let final_zoom = (zoom_f64.floor() - 1.0) as u8;

    final_zoom.clamp(0, MAX_ZOOM)
}

/// Initialisiert Umgebungsvariablen, Datenbank-Pool, HTTP-Client und Signal-Handler.
///
/// Registriert einen `Ctrl+C`-Handler, der das Beenden der Anwendung verzögert,
/// solange ein Datenbank-Schreibvorgang (`in_db`) aktiv ist.
///
/// # Returns
///
/// Ein Tupel aus konfiguriertem [`Client`], dem Laufzeit-Flag (`running`) und dem
/// Datenbank-Aktivitäts-Flag (`in_db`).
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn die Datenbankinitialisierung oder der Bau des HTTP-Clients fehlschlägt.
///
/// # Panics
///
/// Panikt, wenn der `Ctrl+C`-Signal-Handler nicht registriert werden kann.
pub fn init_app() -> Result<(Client, Arc<AtomicBool>, Arc<AtomicBool>)> {
    // 1. .env laden (Plattformunabhängig)
    dotenv().ok();
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
    })
        .expect("Error setting handler");

    Ok((client, running, in_db))
}