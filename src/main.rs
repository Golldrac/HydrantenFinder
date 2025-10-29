mod api;

use crate::api::Coordinate;
use reqwest;
use serde::{Deserialize, Serialize};
use tokio;
use tokio::time::{self, Duration};
use windows::Devices::Geolocation::{
    Geocoordinate, GeolocationAccessStatus, Geolocator, Geoposition, PositionAccuracy,
};
use windows::Foundation::{IAsyncOperation, TimeSpan};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    //let start_lat = 49.79; // Beispiel: Würzburg
    //let start_lon = 9.93;
    //let user_location = Coordinate::new(start_lon, start_lat);
    let user_location = get_user_coordinates().await?;

    println!("{:?}", user_location);

    let hydrants = api::get_nearest_hydrants(user_location, Some(3)).await;

    println!("{:?}", hydrants);

    Ok(())
}

async fn get_user_coordinates() -> Result<Coordinate, Box<dyn std::error::Error>> {
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

            let loc = Coordinate {
                longitude,
                latitude,
            };
            println!("📍 Standort erfolgreich abgerufen!");
            Ok(loc)

        },
        GeolocationAccessStatus::Denied => {
            Err("❌ Standortzugriff wurde in den Windows-Einstellungen verweigert.".into())
        },
        GeolocationAccessStatus::Unspecified => {
            Err("⚠️ Unspezifizierter Fehler beim Abrufen des Standortzugriffs.".into())
        },
        _ => Err("❓ Unbekannter Standortzugriffsstatus.".into()),
    }
}