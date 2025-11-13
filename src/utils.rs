use windows::Devices::Geolocation::{Geocoordinate, GeolocationAccessStatus, Geolocator, Geoposition, PositionAccuracy};
use windows::Foundation::TimeSpan;
use windows_future::IAsyncOperation;
use crate::models::Location;

pub async fn get_user_coordinates() -> anyhow::Result<Location> {
    let access_status = Geolocator::RequestAccessAsync()?.await?;

    match access_status {
        GeolocationAccessStatus::Allowed => {
            println!("✅ Standortzugriff gewährt.");

            let geolocator = Geolocator::new()?;

            geolocator.SetDesiredAccuracy(PositionAccuracy::Default)?;
            let max_age_5_minutes = TimeSpan { Duration: 3_000_000_000 };
            let timeout_5_seconds = TimeSpan { Duration: 50_000_000 };

            let operation: IAsyncOperation<Geoposition> = geolocator
                .GetGeopositionAsyncWithAgeAndTimeout(
                    max_age_5_minutes,
                    timeout_5_seconds
                )?;

            let geoposition = operation.await?;
            let geocoordinate: Geocoordinate = geoposition.Coordinate()?;
            let latitude = geocoordinate.Latitude()?;
            let longitude = geocoordinate.Longitude()?;

            let loc = Location {
                longitude,
                latitude,
            };
            println!("📍 Standort erfolgreich abgerufen!");
            Ok(loc)

        },
        GeolocationAccessStatus::Denied => {
            anyhow::bail!("❌ Standortzugriff wurde in den Windows-Einstellungen verweigert.");
        },
        GeolocationAccessStatus::Unspecified => {
            anyhow::bail!("⚠️ Unspezifizierter Fehler beim Abrufen des Standortzugriffs.");
        },
        _ => anyhow::bail!("❓ Unbekannter Standortzugriffsstatus."),
    }
}

