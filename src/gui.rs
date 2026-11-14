use std::sync::{Arc, Mutex};
use std::time::Duration;
use eframe::egui;
use egui_map_view::{config::OpenStreetMapConfig, Map};

pub struct AppState {
    pub status: String,
    pub show_spinner: bool,
    pub show_map: bool,
    pub show_error_banner: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            status: String::from("Starten ..."),
            show_spinner: true,
            show_map: false,
            show_error_banner: false,
        }
    }
}

pub(crate) struct MyEguiApp {
    state: Arc<Mutex<AppState>>,
    map: Map,
}

impl MyEguiApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, state: Arc<Mutex<AppState>>) -> Self {
        let mut map = Map::new(OpenStreetMapConfig::default());
        map.add_layer(HydrantLayer::new(Arc::clone(&state)));

        Self { state, map }
    }
}

impl eframe::App for MyEguiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.ctx().request_repaint_after(Duration::from_millis(100));

        // Zustand kurz auslesen (oder bei wenig Inhalt direkt gesperrt halten)
        let (status, show_spinner, show_map, show_error) = {
            let s = self.state.lock().unwrap();
            (s.status.clone(), s.show_spinner, s.show_map, s.show_error_banner)
        };

        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                if show_spinner {
                    ui.spinner();
                }
                ui.heading(&status);
            });

            if show_error {
                ui.colored_label(egui::Color32::RED, "[!] Bitte Internetverbindung oder GPS prüfen.");
            }

            if show_map {
                ui.add_space(10.0);
                ui.add(&mut self.map);
            }
        });
    }
}