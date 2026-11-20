use std::any::Any;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use eframe::egui::Color32;
use eframe::egui;
use eframe::egui::Response;
use eframe::epaint::PathStroke;
use egui_map_view::{config::OpenStreetMapConfig, Map, layers::{svg, Layer}, projection};
use egui_map_view::projection::{GeoPos, MapProjection};
use geo_types::Coord;
use crate::utils::calculate_zoom_level;
use crate::models::{HydrantWithRoute, Location};
use crate::api::POLYLINE_PRECISION;


struct DrawLayer {
    user_loc: Option<Location>,
    hydrants: Vec<HydrantWithRoute>,
    polylines: Vec<Vec<Coord>>,
}

impl Layer for DrawLayer {
//Mögliche fraben: BLACK, DARK_GRAY, GRAY, LIGHT_GRAY, WHITE, BROWN, DARK_RED, RED, LIGHT_RED, CYAN, MAGENTA, YELLOW, ORANGE, LIGHT_YELLOW, KHAKI, DARK_GREEN, GREEN, LIGHT_GREEN, DARK_BLUE, BLUE, LIGHT_BLUE, PURPLE, GOLD, from_rgb()
    fn draw(&self, painter: &egui::Painter, projection: &MapProjection) {
        for i in 0..self.polylines.len() {
            let hyd_lines = &self.polylines[i];
            let col = &self.hydrants[i].color;
            for j in 1 .. hyd_lines.len() {
                let screen_pos_1 = projection.project(GeoPos{
                    lat: hyd_lines[j].y,
                    lon: hyd_lines[j].x
                });
                let screen_pos_2 = projection.project(GeoPos{
                    lat: hyd_lines[j-1].y,
                    lon: hyd_lines[j-1].x
                });
                painter.line([screen_pos_1, screen_pos_2].to_vec(), PathStroke::new(9.0 - (i as f32 *2.0), *col));
            }

        }


        if let Some(loc) = &self.user_loc {
            let pos = GeoPos {
                lat: loc.latitude,
                lon: loc.longitude,
            };

            let screen_pos = projection.project(pos);
            painter.circle(
                screen_pos,
                7.0,
                Color32::BLACK,
                egui::Stroke::new(2.0, Color32::BLACK),
            );
        }


        for hydrant in &self.hydrants {
            let pos = GeoPos {
                lat: hydrant.hydrant.latitude,
                lon:  hydrant.hydrant.longitude,
            };

            let screen_pos = projection.project(pos);
            painter.circle(
                screen_pos,
                7.0,
                hydrant.color,
                egui::Stroke::new(2.0, hydrant.color),
            );
        }

    }
    fn handle_input(&mut self, _response: &egui::Response, _projection: &MapProjection) -> bool {
        false
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

}

impl DrawLayer {
    fn new()-> DrawLayer {
        DrawLayer {
            user_loc: None,
            hydrants: Vec::new(),
            polylines: Vec::new(),
        }
    }

    fn set_user_loc(&mut self, user_loc: Location){
        self.user_loc = Some(user_loc);
    }

    fn set_hydrants(&mut self, hydrants: Vec<HydrantWithRoute>) {
        self.polylines.clear();
        self.hydrants = hydrants;

        for hydrant in &self.hydrants {
            let poly_str = &hydrant.route_details.geometry_polyline;
            if !poly_str.is_empty() {
                if let Ok(line_string) = polyline::decode_polyline(poly_str, POLYLINE_PRECISION) {
                    self.polylines.push(line_string.0);
                }
            }
        }
    }
}


pub struct AppState {
    is_loading: bool,
    show_error_banner: bool,
    error_msg: String,
    user_loc: Option<Location>,
    hydrants: Vec<HydrantWithRoute>,
    current_hydrant: Option<usize>,
    center_needs_update: bool,
    zoom_needs_update: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            is_loading: true,
            show_error_banner: false,
            error_msg: "Unbekannter Fehler!".into(),
            user_loc: None,
            hydrants: Vec::new(),
            current_hydrant: None,
            center_needs_update: false,
            zoom_needs_update: false,
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
        self.center_needs_update = true;
    }

    pub fn set_hydrants(&mut self, hydrants: Vec<HydrantWithRoute>){
        //Mögliche fraben: BLACK, DARK_GRAY, GRAY, LIGHT_GRAY, WHITE, BROWN, DARK_RED, RED, LIGHT_RED, CYAN, MAGENTA, YELLOW, ORANGE, LIGHT_YELLOW, KHAKI, DARK_GREEN, GREEN, LIGHT_GREEN, DARK_BLUE, BLUE, LIGHT_BLUE, PURPLE, GOLD, from_rgb()
        let available_colors = [Color32::RED, Color32::BLUE, Color32::MAGENTA, Color32::ORANGE, Color32::CYAN, Color32::GREEN];
        self.hydrants = hydrants;
        self.zoom_needs_update = true;

        for (i, hydrant) in self.hydrants.iter_mut().enumerate() {
            hydrant.color = available_colors[i % available_colors.len()];
        }
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
        map.add_layer("draw_layer", DrawLayer::new());
        Self { state, map}
    }
}

impl eframe::App for MyEguiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.ctx().request_repaint_after(Duration::from_millis(100));

        let (is_loading, show_error_banner, user_loc, hydrants, current_hydrant, error_msg, update_center, update_zoom) = {
            let mut s = self.state.lock().unwrap();
            let uc = s.center_needs_update;
            let uz = s.zoom_needs_update;
            s.center_needs_update = false;
            s.zoom_needs_update = false;
            (s.is_loading, s.show_error_banner, s.user_loc, s.hydrants.clone(), s.current_hydrant, s.error_msg.clone(), uc, uz)
        };

        if !is_loading && !show_error_banner {

            egui::TopBottomPanel::top("top_panel").show_inside(ui, |ui| {
                ui.heading("Top Panel Zeug");
            });
            egui::SidePanel::right("side_panel").show_inside(ui, |ui| {
                ui.heading("Hydranten");
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for i in 0.. hydrants.len()  {
                        let hydrant = &hydrants[i];
                        ui.colored_label(hydrants[i].color, hydrants[i].hydrant.tags.as_ref().expect("Keine Infos zu diesem Hydranten").output());
                    }
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
                ui.colored_label(Color32::RED, error_msg);
            }

            if !is_loading && !show_error_banner {


                if update_center {

                    if let Some(user) = user_loc {
                        if let Some(layer) = self.map.layer_mut::<DrawLayer>("draw_layer") {
                            layer.set_user_loc(user);
                        }
                        self.map.center.lat = user.latitude;
                        self.map.center.lon = user.longitude;
                    }
                }

                if update_zoom {
                    if let (Some(user), Some(first_hydrant)) = (user_loc, &hydrants.first()) {
                        if let Some(layer) = self.map.layer_mut::<DrawLayer>("draw_layer") {
                            layer.set_hydrants(hydrants.clone());
                        }
                        self.map.zoom = calculate_zoom_level(&user, &first_hydrant.hydrant);
                    }
                }

                ui.add_sized(ui.available_size_before_wrap(), &mut self.map);

            }
        });
    }
}