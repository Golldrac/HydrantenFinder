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

            let loc = Location {
                longitude: geocoordinate.Longitude()?,
                latitude: geocoordinate.Latitude()?,
            };

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