use anyhow::Result;
use image::{GenericImageView, EncodableLayout};
use minifb::{Window, WindowOptions, Key};
use std::path::Path;
use windows::Devices::Geolocation::{Geocoordinate, GeolocationAccessStatus, Geolocator, Geoposition, PositionAccuracy};
use windows::Foundation::TimeSpan;
use windows_future::IAsyncOperation;
use crate::models::Location;

/// Konvertiert Sekunden in die 100-Nanosekunden-Einheit, die für `TimeSpan::Duration` benötigt wird.
const DURATION_FACTOR: i64 = 10_000_000; // 1 Sekunde = 10 Millionen * 100ns

/// Die maximale Dauer, für die eine gespeicherte Position akzeptiert wird (5 Minuten).
const MAX_AGE_DURATION_NANOS: i64 = 5 * 60 * DURATION_FACTOR;

/// Der Timeout für die Positionsanforderung (5 Sekunden).
const TIMEOUT_DURATION_NANOS: i64 = 5 * DURATION_FACTOR;

/// Der Skalierungsfaktor, der auf das geladene Bild angewendet wird.
///
/// Ein Wert unter 1.0 bewirkt ein **Herauszoomen** (Bild wird verkleinert, um auf den Bildschirm zu passen),
/// während ein Wert über 1.0 ein **Hineinzoomen** bewirkt.
const SCALE_FACTOR: f32 = 0.8;

/// Ruft die aktuellen geographischen Koordinaten des Benutzers über die Windows Geolocation API ab.
///
/// Stellt sicher, dass der Standortzugriff gewährt ist und versucht,
/// eine Position mit einem maximalen Alter von 5 Minuten und einem Timeout von 5 Sekunden zu erhalten.
///
/// # Errors
/// Gibt einen Fehler zurück, wenn:
/// * Der Standortzugriff in den Windows-Einstellungen verweigert wurde.
/// * Die Windows API einen Fehler zurückgibt (z.B. Timeout oder interner Fehler).
pub async fn get_user_coordinates() -> anyhow::Result<Location> {
    let access_status = Geolocator::RequestAccessAsync()?.await?;

    match access_status {
        GeolocationAccessStatus::Allowed => {

            let geolocator = Geolocator::new()?;
            geolocator.SetDesiredAccuracy(PositionAccuracy::Default)?;

            let max_age = TimeSpan { Duration: MAX_AGE_DURATION_NANOS };
            let timeout = TimeSpan { Duration: TIMEOUT_DURATION_NANOS };

            let operation: IAsyncOperation<Geoposition> = geolocator
                .GetGeopositionAsyncWithAgeAndTimeout(
                    max_age,
                    timeout
                )?;

            let geoposition = operation.await?;
            let geocoordinate: Geocoordinate = geoposition.Coordinate()?;

            let loc = Location::new(geocoordinate.Longitude()?, geocoordinate.Latitude()?);

            Ok(loc)
        },
        GeolocationAccessStatus::Denied => {
            anyhow::bail!("❌ Standortzugriff wurde in den Windows-Einstellungen verweigert.");
        },
        GeolocationAccessStatus::Unspecified => {
            anyhow::bail!("⚠️ Unspezifizierter Fehler beim Abrufen des Standortzugriffs.");
        },
        // Deckt zukünftige/unbekannte Status ab.
        _ => anyhow::bail!("❓ Unbekannter Standortzugriffsstatus."),
    }
}

/// Lädt ein Bild von einem Pfad, skaliert es (entsprechend `SCALE_FACTOR`)
/// und zeigt es in einem `minifb`-Fenster an.
///
/// Das Bild wird auf das **ARGB-Format (u32)** für `minifb` konvertiert und
/// in einer Ereignisschleife angezeigt, bis der Benutzer **ESC** drückt oder das Fenster schließt.
///
/// # Arguments
///
/// * `file_path` - Der Pfad zur Bilddatei (z.B. "hydrant_route.png").
///
/// # Errors
///
/// Gibt einen Fehler zurück, wenn das Bild nicht geladen/dekodiert werden kann oder
/// `minifb`-Operationen fehlschlagen.
pub(crate) fn show_image_in_minifb(file_path: &str) -> Result<()> {
    // 1. Das Bild laden und dekodieren
    let img = image::open(Path::new(file_path))?;

    // Berechne die neuen Abmessungen basierend auf dem globalen Skalierungsfaktor
    let new_width = (img.width() as f32 * SCALE_FACTOR) as u32;
    let new_height = (img.height() as f32 * SCALE_FACTOR) as u32;

    // Skaliere das Bild auf die neue Größe (Herauszoomen).
    // Die Methode Lanczos3 bietet eine hohe Qualität für Verkleinerungen.
    let resized_img = img.resize_exact(
        new_width,
        new_height,
        image::imageops::FilterType::Lanczos3
    );

    // Abmessungen des *skalierten* Bildes abrufen
    let width = resized_img.width() as usize;
    let height = resized_img.height() as usize;

    // 2. Den Pixelpuffer vorbereiten
    // 'minifb' benötigt einen Puffer von u32-Werten, wobei jeder u32
    // die 32-Bit-Farbinformation (ARGB) eines Pixels darstellt.
    let mut buffer: Vec<u32> = vec![0; width * height];

    // Iteriere über die Pixel des skalierten Bildes und konvertiere sie in das u32-Format.
    for (x, y, pixel) in resized_img.pixels() {
        // 'pixel.0' ist ein Array von [r, g, b, a] oder [r, g, b]
        let (r, g, b, a) = match pixel.0.len() {
            4 => (pixel.0[0] as u32, pixel.0[1] as u32, pixel.0[2] as u32, pixel.0[3] as u32),
            3 => (pixel.0[0] as u32, pixel.0[1] as u32, pixel.0[2] as u32, 255u32), // Keine Alpha-Werte (Opak)
            _ => continue, // Nicht unterstütztes Format
        };

        // ARGB-Format: (Alpha << 24) | (Rot << 16) | (Grün << 8) | (Blau)
        let color = (a << 24) | (r << 16) | (g << 8) | b;

        // Berechne den Index im flachen Puffer
        let index = (y as usize) * width + (x as usize);
        buffer[index] = color;
    }

    // 3. Das Fenster erstellen
    let mut window = Window::new(
        "Rust Bildanzeige (ESC zum Schließen)",
        width,
        height,
        // Optionen für das Fenster
        WindowOptions {
            // minifb passt das Fenster an die Bildschirmauflösung an.
            scale: minifb::Scale::FitScreen,
            ..WindowOptions::default() // Übernimmt den Rest der Standardeinstellungen
        },
    )?;

    // Begrenze die Aktualisierungsrate des Fensters auf 60 FPS
    window.limit_update_rate(Some(std::time::Duration::from_millis(1000 / 60)));

    // 4. Die Ereignisschleife (Loop)
    while window.is_open() && !window.is_key_down(Key::Escape) {
        // Aktualisiere den Fensterinhalt mit dem Pixelpuffer
        window.update_with_buffer(&buffer, width, height)?;
    }

    Ok(())
}