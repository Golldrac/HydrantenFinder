mod api;
mod models;


use crate::models::Location;
use tokio;
use windows::Devices::Geolocation::{
    Geocoordinate, GeolocationAccessStatus, Geolocator, PositionAccuracy, Geoposition,
};
use windows::Foundation::TimeSpan;
use windows_future::IAsyncOperation;
use dotenvy::dotenv;
use anyhow::Result;

const POLYLINE_PRECISION: u32 = 5;

#[tokio::main]
async fn main() -> Result<()> {
    //let start_lat = 49.79; // Beispiel: Würzburg
    //let start_lon = 9.93;
    //let user_location = Coordinate::new(start_lon, start_lat);
    dotenv().ok();
    let user_location = get_user_coordinates().await?;
    println!("{:?}", user_location);

    let hydrants = api::get_nearest_hydrants(&user_location, Some(5)).await.expect("REASON");
    if hydrants.is_empty() {
        println!("Keine Hydranten in der Nähe gefunden. Das Programm wird beendet.");
        return Ok(());
    }
    println!("{:?}", hydrants);

    let best_hydrant = api::get_closest_by_route(&user_location, &hydrants).await?;

    println!("\n⭐ Nächster Hydrant (nach Fußweg) gefunden!");
    println!("  Koordinaten: ({:.6}, {:.6})", best_hydrant.location.latitude, best_hydrant.location.longitude);
    println!("  Wegstrecke: {:.2} Meter", best_hydrant.route_details.distance_meters);
    println!("  Routen-Geometrie (Polyline): {}", best_hydrant.route_details.geometry_polyline);

    match api::decode_route_polyline(&best_hydrant.route_details.geometry_polyline, POLYLINE_PRECISION) {
        Ok(route_path) => {
            println!("\n✅ Routenpfad erfolgreich dekodiert ({} Punkte):", route_path.len());
            let display_count = route_path.len().min(3);
            for (i, loc) in route_path.iter().take(display_count).enumerate() {
                println!("  Startpunkt {}/{} (Lon: {:.6}, Lat: {:.6})", i+1, route_path.len(), loc.longitude, loc.latitude);
            }

            if route_path.len() > 6 {
                println!("  ...");
            }

            for (i, loc) in route_path.iter().skip(route_path.len().saturating_sub(display_count)).enumerate() {
                println!("  Endpunkt {}/{} (Lon: {:.6}, Lat: {:.6})", route_path.len() - display_count + i + 1, route_path.len(), loc.longitude, loc.latitude);
            }

        },
        Err(e) => {
            eprintln!("\n❌ Fehler beim Dekodieren der Polyline: {}", e);
        }
    }

    Ok(())
}

async fn get_user_coordinates() -> Result<Location> {
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