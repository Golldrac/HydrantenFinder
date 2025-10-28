mod api;

use reqwest;
use tokio;
use serde::{Deserialize, Serialize};
use windows::Devices::Geolocation::{
    GeolocationAccessStatus, Geolocator, PositionAccuracy,
    Geoposition, Geocoordinate
};
use windows::Foundation::IAsyncOperation;
use crate::api::Coordinate;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let start_lat = 49.79; // Beispiel: Würzburg
    let start_lon = 9.93;

    //let hydrants = api::find_hydrants(start_lat, start_lon, 1000).await?;
    //println!("Gefundene Hydranten: {}", hydrants.len());
    let result = get_user_coordinates().await?;

    println!("{:?}", result);

    Ok(())
}

async fn get_user_coordinates() -> Result<Coordinate, Box<dyn std::error::Error>> {
    let access_status = Geolocator::RequestAccessAsync()?.await?;

    match access_status {
        GeolocationAccessStatus::Allowed => {
            println!("✅ Standortzugriff gewährt.");

            let geolocator = Geolocator::new()?;
            geolocator.SetDesiredAccuracy(PositionAccuracy::High)?;
            let operation: IAsyncOperation<Geoposition> = geolocator.GetGeopositionAsync()?;
            let geoposition: Geoposition = operation.await?;
            let geocoordinate: Geocoordinate = geoposition.Coordinate()?;
            let latitude = geocoordinate.Latitude()?;
            let longitude = geocoordinate.Longitude()?;

            let loc = Coordinate {
                longitude,
                latitude,
            };

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