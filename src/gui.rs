use std::sync::{Arc, Mutex};
use std::time::Duration;
use eframe::egui;
use egui_map_view::{config::OpenStreetMapConfig, Map};
use crate::models::{HydrantWithRoute, Location};

pub struct AppState {
    //pub status: String,
    is_loading: bool,
    //pub show_spinner: bool,
    //pub show_map: bool,
    show_error_banner: bool,
    error_msg: String,
    user_loc: Option<Location>,
    hydrants: Vec<HydrantWithRoute>,
    current_hydrant: Option<usize>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            //status: String::from("Starten ..."),
            is_loading: true,
            //show_spinner: true,
            //show_map: false,
            show_error_banner: false,
            error_msg: "Unbekannter Fehler!".into(),
            user_loc: None,
            hydrants: Vec::new(),
            current_hydrant: None,
        }
    }

}

impl AppState {

    pub fn set_loading(&mut self, loading: bool){
        self.is_loading = loading;
    }
    pub fn set_error_banner(&mut self, error_banner: bool){
        self.show_error_banner = error_banner;
    }

    pub fn set_user_loc(&mut self, user_loc: Location){
        self.user_loc = Some(user_loc);
    }

    pub fn set_hydrants(&mut self, hydrants: Vec<HydrantWithRoute>){
        self.hydrants = hydrants;
    }

    pub fn set_hydrant(&mut self, hydrant: usize) {
        self.current_hydrant = Some(hydrant);
    }

    pub fn set_error_msg(&mut self, msg: String){
        self.error_msg = msg;
    }

    pub fn is_loading(&self) -> bool{
        return self.is_loading;
    }

    pub fn get_show_error_banner(&self) -> bool{
        return self.show_error_banner;
    }
    pub fn get_user_loc(&self) -> Option<Location>{
        return self.user_loc;
    }

    pub fn get_hydrants(&self) -> &Vec<HydrantWithRoute>{
        return &self.hydrants;
    }

    pub fn get_current_hydrant(&self) -> Option<usize>{
        return self.current_hydrant;
    }
    pub fn get_error_msg(&self) -> String{
        return self.error_msg.clone();
    }

}

pub(crate) struct MyEguiApp {
    state: Arc<Mutex<AppState>>,
    map: Map,
}

impl MyEguiApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, state: Arc<Mutex<AppState>>) -> Self {
        let mut map = Map::new(OpenStreetMapConfig::default());
        Self { state, map }
    }
}

impl eframe::App for MyEguiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.ctx().request_repaint_after(Duration::from_millis(100));

        let (is_loading, show_error_banner, user_loc, hydrants, current_hydrant, error_msg) = {
            let s = self.state.lock().unwrap();
            (s.is_loading, s.show_error_banner, s.user_loc, s.hydrants.clone(), s.current_hydrant, s.error_msg.clone())
        };

        if !is_loading && !show_error_banner {

            egui::TopBottomPanel::top("top_panel").show_inside(ui, |ui| {
                ui.heading("Top Panel Zeug");
            });
            egui::SidePanel::right("side_panel").show_inside(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Side Panel Zeug");
                });
            });


        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                if is_loading {
                    ui.spinner();
                    ui.heading("Loading ...");
                }

            });

            if show_error_banner  {
                ui.colored_label(egui::Color32::RED, error_msg);
            }

            if !is_loading && !show_error_banner {
                ui.add_space(10.0);
                ui.add(&mut self.map);
            }
        });
    }
}