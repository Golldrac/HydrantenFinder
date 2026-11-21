use std::any::Any;
use std::time::Duration;
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32};
use eframe::epaint::PathStroke;
use egui_map_view::config::OpenStreetMapConfig;
use egui_map_view::layers::Layer;
use egui_map_view::Map;
use egui_map_view::projection::{GeoPos, MapProjection};
use geo_types::Coord;

use crate::api::POLYLINE_PRECISION;
use crate::models::{HydrantPosition, HydrantWithRoute, Location};
use crate::utils::calculate_zoom_level;

/// Karten-Overlay-Layer zum Zeichnen von Benutzerposition, Hydrantenmarkern und Routenlinien.
struct DrawLayer {
    /// Aktuelle Benutzerposition (falls bereits ermittelt).
    user_loc: Option<Location>,
    /// Liste der darzustellenden Hydranten inklusive Routen und Farben.
    hydrants: Vec<HydrantWithRoute>,
    /// Dekodierte Polyline-Koordinaten für jeden Hydranten.
    polylines: Vec<Vec<Coord>>,
    /// Index des aktuell ausgewählten Hydranten für die Routenhervorhebung.
    current_hydrant: Option<usize>,
}

impl Layer for DrawLayer {
    /// Zeichnet die aktive Route, den Benutzerstandort und alle Hydrantenmarker auf die Karte.
    ///
    /// # Arguments
    ///
    /// * `painter` - Der `egui::Painter` zum Rendern der Vektorelemente.
    /// * `projection` - Die aktuelle Kartenprojektion zur Umrechnung von Geo- in Bildschirmkoordinaten.
    fn draw(&self, painter: &egui::Painter, projection: &MapProjection) {
        if let Some(chosen) = self.current_hydrant {
            let hyd_lines = &self.polylines[chosen];
            let col = &self.hydrants[chosen].color;

            if hyd_lines.len() == 0 {
                if let Some(loc) = &self.user_loc {
                    let user_pos = projection.project(GeoPos {
                        lat: loc.latitude,
                        lon: loc.longitude,
                    });

                    let hyd_pos = projection.project(GeoPos {
                        lat: self.hydrants[chosen].hydrant.latitude,
                        lon: self.hydrants[chosen].hydrant.longitude,
                    });
                    painter.line([user_pos, hyd_pos].to_vec(), PathStroke::new(6.0_f32, *col));
                }
            }

            for j in 1..hyd_lines.len() {
                let screen_pos_1 = projection.project(GeoPos {
                    lat: hyd_lines[j].y,
                    lon: hyd_lines[j].x,
                });
                let screen_pos_2 = projection.project(GeoPos {
                    lat: hyd_lines[j - 1].y,
                    lon: hyd_lines[j - 1].x,
                });
                painter.line(
                    [screen_pos_1, screen_pos_2].to_vec(),
                    PathStroke::new(6.0_f32, *col),
                );
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
                egui::Stroke::new(2.0_f32, Color32::BLACK),
            );
        }

        for hydrant in &self.hydrants {
            let screen_pos = projection.project(GeoPos {
                lat: hydrant.hydrant.latitude,
                lon: hydrant.hydrant.longitude,
            });
            painter.circle(
                screen_pos,
                7.0,
                hydrant.color,
                egui::Stroke::new(2.0_f32, hydrant.color),
            );
        }
    }

    /// Verarbeitet Klicks auf die Karte und wählt angeklickte Hydrantenmarker aus.
    ///
    /// # Arguments
    ///
    /// * `response` - Die `egui::Response` des Karten-Widgets.
    /// * `projection` - Die aktuelle Kartenprojektion.
    ///
    /// # Returns
    ///
    /// `true`, wenn ein Hydrant angeklickt und ausgewählt wurde, andernfalls `false`.
    fn handle_input(&mut self, response: &egui::Response, projection: &MapProjection) -> bool {
        if response.clicked() {
            if let Some(click_pos) = response.interact_pointer_pos() {
                for (i, hydrant) in self.hydrants.iter().enumerate().rev() {
                    let screen_pos = projection.project(GeoPos {
                        lat: hydrant.hydrant.latitude,
                        lon: hydrant.hydrant.longitude,
                    });

                    if click_pos.distance(screen_pos) <= 10.0 {
                        self.current_hydrant = Some(i);
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Gibt eine unveränderliche [`Any`]-Referenz für Downcasts zurück.
    ///
    /// # Returns
    ///
    /// `&dyn Any` auf `self`.
    fn as_any(&self) -> &dyn Any {
        self
    }

    /// Gibt eine veränderliche [`Any`]-Referenz für Downcasts zurück.
    ///
    /// # Returns
    ///
    /// `&mut dyn Any` auf `self`.
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl DrawLayer {
    /// Erstellt eine neue, leere [`DrawLayer`]-Instanz.
    ///
    /// # Returns
    ///
    /// Eine initialisierte [`DrawLayer`]-Instanz ohne Standort und Hydranten.
    fn new() -> DrawLayer {
        DrawLayer {
            user_loc: None,
            hydrants: Vec::new(),
            polylines: Vec::new(),
            current_hydrant: None,
        }
    }

    /// Setzt die Benutzerposition für die Kartendarstellung.
    ///
    /// # Arguments
    ///
    /// * `user_loc` - Die neue [`Location`] des Benutzers.
    fn set_user_loc(&mut self, user_loc: Location) {
        self.user_loc = Some(user_loc);
    }

    /// Aktualisiert die Hydrantenliste, dekodiert deren Polylines und wählt den ersten Hydranten aus.
    ///
    /// # Arguments
    ///
    /// * `hydrants` - Die neue Liste von [`HydrantWithRoute`].
    fn set_hydrants(&mut self, hydrants: Vec<HydrantWithRoute>) {
        self.polylines.clear();
        self.hydrants = hydrants;
        self.current_hydrant = Some(0usize);

        for hydrant in &self.hydrants {
            let poly_str = &hydrant.route_details.geometry_polyline;
            if !poly_str.is_empty() {
                if let Ok(line_string) = polyline::decode_polyline(poly_str, POLYLINE_PRECISION) {
                    self.polylines.push(line_string.0);
                }
            }
        }
    }

    /// Setzt den Index des Hydranten, dessen Route auf der Karte hervorgehoben wird.
    ///
    /// # Arguments
    ///
    /// * `current_hyd` - Der Index des Hydranten in `self.hydrants`.
    fn set_current_hydrant(&mut self, current_hyd: usize) {
        self.current_hydrant = Some(current_hyd);
    }
}

/// Geteilter Anwendungszustand zwischen dem Hintergrund-Suchtask und dem GUI-Thread.
pub struct AppState {
    /// Gibt an, ob die Standort- oder Hydrantensuche noch läuft.
    is_loading: bool,
    /// Steuert, ob eine Fehlermeldung statt der Karte angezeigt wird.
    show_error_banner: bool,
    /// Text der anzuzeigenden Fehlermeldung.
    error_msg: String,
    /// Ermittelter Benutzerstandort.
    user_loc: Option<Location>,
    /// Ermittelte und eingefärbte Hydranten inklusive Routendetails.
    hydrants: Vec<HydrantWithRoute>,
    /// Signalisiert der GUI, das Kartenzentrum einmalig auf den Benutzerstandort zu setzen.
    center_needs_update: bool,
    /// Signalisiert der GUI, den Karten-Zoomlevel einmalig an den ersten Hydranten anzupassen.
    zoom_needs_update: bool,
}

impl Default for AppState {
    /// Erstellt den Standardzustand beim Programmstart (`is_loading = true`).
    ///
    /// # Returns
    ///
    /// Eine initialisierte [`AppState`]-Instanz.
    fn default() -> Self {
        Self {
            is_loading: true,
            show_error_banner: false,
            error_msg: "Unbekannter Fehler!".into(),
            user_loc: None,
            hydrants: Vec::new(),
            center_needs_update: false,
            zoom_needs_update: false,
        }
    }
}

impl AppState {
    /// Setzt den Ladezustand der Anwendung.
    ///
    /// # Arguments
    ///
    /// * `loading` - `true`, solange Hintergrundabfragen aktiv sind.
    pub fn set_loading(&mut self, loading: bool) {
        self.is_loading = loading;
    }

    /// Aktiviert oder deaktiviert die Fehleranzeige in der GUI.
    ///
    /// # Arguments
    ///
    /// * `error_banner` - `true`, um das Fehlerbanner anzuzeigen.
    pub fn set_error_banner(&mut self, error_banner: bool) {
        self.show_error_banner = error_banner;
    }

    /// Speichert den Benutzerstandort und markiert das Kartenzentrum zur Aktualisierung.
    ///
    /// # Arguments
    ///
    /// * `user_loc` - Die ermittelte [`Location`] des Benutzers.
    pub fn set_user_loc(&mut self, user_loc: Location) {
        self.user_loc = Some(user_loc);
        self.center_needs_update = true;
    }

    /// Speichert die gefundenen Hydranten, weist ihnen unterscheidbare Farben zu und
    /// markiert den Karten-Zoom zur Aktualisierung.
    ///
    /// # Arguments
    ///
    /// * `hydrants` - Die Liste der berechneten [`HydrantWithRoute`].
    pub fn set_hydrants(&mut self, hydrants: Vec<HydrantWithRoute>) {
        let available_colors = [
            Color32::from_rgb(255, 95, 95),
            Color32::ORANGE,
            Color32::YELLOW,
            Color32::from_rgb(85, 230, 115),
            Color32::CYAN,
            Color32::from_rgb(85, 175, 255),
            Color32::from_rgb(185, 130, 255),
            Color32::from_rgb(255, 115, 205),
        ];

        self.hydrants = hydrants;
        self.zoom_needs_update = true;

        for (i, hydrant) in self.hydrants.iter_mut().enumerate() {
            hydrant.color = available_colors[i % available_colors.len()];
        }
    }

    /// Setzt den anzuzeigenden Fehlertext.
    ///
    /// # Arguments
    ///
    /// * `msg` - Die Fehlermeldung als `String`.
    pub fn set_error_msg(&mut self, msg: String) {
        self.error_msg = msg;
    }
}

/// Haupt-GUI-Anwendung für `eframe` mit Kartenansicht und Seitenleiste.
pub(crate) struct MyEguiApp {
    /// Geteilter, thread-sicherer Anwendungszustand.
    state: Arc<Mutex<AppState>>,
    /// Interaktive OpenStreetMap-Kartenkomponente.
    map: Map,
}

impl MyEguiApp {
    /// Erstellt eine neue [`MyEguiApp`]-Instanz und registriert den [`DrawLayer`] auf der Karte.
    ///
    /// # Arguments
    ///
    /// * `_cc` - Der `eframe::CreationContext` (unbenutzt).
    /// * `state` - Der geteilte [`AppState`].
    ///
    /// # Returns
    ///
    /// Eine initialisierte [`MyEguiApp`]-Instanz.
    pub fn new(_cc: &eframe::CreationContext<'_>, state: Arc<Mutex<AppState>>) -> Self {
        let mut map = Map::new(OpenStreetMapConfig::default());
        map.add_layer("draw_layer", DrawLayer::new());
        Self { state, map }
    }
}

impl eframe::App for MyEguiApp {
    /// Rendert die gesamte Benutzeroberfläche pro Frame (Top-Panel, Seitenleiste und Karte).
    ///
    /// # Arguments
    ///
    /// * `ui` - Der aktuelle `egui::Ui`-Kontext.
    /// * `_frame` - Der `eframe::Frame` (unbenutzt).
    ///
    /// # Panics
    ///
    /// Panikt, wenn der `Mutex` von `self.state` vergiftet (poisoned) ist.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.ctx().request_repaint_after(Duration::from_millis(100));

        let (is_loading, show_error_banner, user_loc, hydrants, error_msg, update_center, update_zoom) = {
            let mut s = self.state.lock().unwrap();
            let uc = s.center_needs_update;
            let uz = s.zoom_needs_update;
            s.center_needs_update = false;
            s.zoom_needs_update = false;
            (
                s.is_loading,
                s.show_error_banner,
                s.user_loc,
                s.hydrants.clone(),
                s.error_msg.clone(),
                uc,
                uz,
            )
        };

        if !is_loading && !show_error_banner {
            egui::TopBottomPanel::top("top_panel").show_inside(ui, |ui| {
                //TODO
                ui.heading("Top Panel Zeug");
            });
            egui::SidePanel::right("side_panel")
                .default_size(350.0)
                .show_inside(ui, |ui| {
                    ui.label(egui::RichText::new("Hydranten\n\n").size(30.0));
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for i in 0..hydrants.len() {
                            let hydrant = &hydrants[i];
                            let mut text = String::new();

                            text.push_str(
                                format!(
                                    "Der Hydrant ist {}m entfernt.\n",
                                    hydrant.route_details.distance_meters
                                )
                                    .as_str(),
                            );
                            if let Some(tags) = &hydrant.hydrant.tags {
                                if let Some(shape) = &tags.shape {
                                    text.push_str(
                                        format!(
                                            "Es handelt sich um einen {}.\n",
                                            shape.to_display_name()
                                        )
                                            .as_str(),
                                    );
                                }
                                if let Some(diameter) = &tags.diameter {
                                    text.push_str(
                                        format!(
                                            "Das Rohr hat einen Durchmesser von {}mm\n",
                                            diameter
                                        )
                                            .as_str(),
                                    );
                                }
                                if let Some(position) = &tags.position {
                                    match position {
                                        HydrantPosition::Green => {
                                            text.push_str(
                                                "Der Hydrant befindet sich auf einer Grünfläche.\n",
                                            );
                                        }
                                        HydrantPosition::Lane => {
                                            text.push_str(
                                                "Der Hydrant befindet sich auf der Straße.\n",
                                            );
                                        }
                                        HydrantPosition::ParkingLot => {
                                            text.push_str(
                                                "Der Hydrant befindet sich auf einem Parkplatz.\n",
                                            );
                                        }
                                        HydrantPosition::Sidewalk => {
                                            text.push_str(
                                                "Der Hydrant befindet sich auf dem Gehweg.\n",
                                            );
                                        }
                                    }
                                }

                                if let Some(couplings) = &tags.couplings {
                                    text.push_str(
                                        format!("Der Hydrant hat {} Kupplung(en).\n", couplings)
                                            .as_str(),
                                    );
                                }
                            }
                            let response = ui
                                .colored_label(hydrants[i].color, text)
                                .interact(egui::Sense::click());

                            if response.clicked() {
                                if let Some(layer) = self.map.layer_mut::<DrawLayer>("draw_layer") {
                                    layer.set_current_hydrant(i);
                                }
                            }
                        }
                    });
                });
        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                if is_loading {
                    //TODO: Schöner
                    ui.spinner();
                    ui.heading("Loading ...");
                }
            });

            if show_error_banner {
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