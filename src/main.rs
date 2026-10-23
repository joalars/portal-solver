mod pathfinding;

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Sense, Stroke, Vec2};
use pathfinding::{Direction, PortalExits, Route, shortest_path};

const GRID_SIZE: usize = 5;

struct PortalFile {
    name: String,
    graph: Result<BTreeMap<u64, PortalExits>, String>,
}

struct PathfinderApp {
    files: Vec<PortalFile>,
    selected_file: Option<usize>,
    start: String,
    destination: String,
    route: Option<Route>,
    message: String,
    mode: AppMode,
    next_click_is_start: bool,
    editor_graph: BTreeMap<u64, PortalExits>,
    editor_selected: u64,
    editor_targets: [String; 4],
    editor_filename: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Pathfinder,
    Editor,
}

impl PathfinderApp {
    fn new() -> Self {
        let mut app = Self {
            files: Vec::new(),
            selected_file: None,
            start: String::new(),
            destination: String::new(),
            route: None,
            message: String::new(),
            mode: AppMode::Pathfinder,
            next_click_is_start: true,
            editor_graph: empty_map(),
            editor_selected: 11,
            editor_targets: std::array::from_fn(|_| "0".to_owned()),
            editor_filename: String::new(),
        };
        app.reload_files();
        app
    }

    fn reload_files(&mut self) {
        self.route = None;
        match find_portal_directory() {
            Ok(directory) => match fs::read_dir(&directory) {
                Ok(entries) => {
                    let mut paths: Vec<PathBuf> = entries
                        .filter_map(Result::ok)
                        .map(|entry| entry.path())
                        .filter(|path| {
                            path.extension()
                                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
                        })
                        .collect();
                    paths.sort();
                    self.files = paths
                        .into_iter()
                        .map(|path| PortalFile::load(&path))
                        .collect();
                    self.selected_file = (!self.files.is_empty()).then_some(0);
                    self.message = if self.files.is_empty() {
                        format!("No .json files found in {}", directory.display())
                    } else {
                        format!("Loaded {} portal file(s).", self.files.len())
                    };
                }
                Err(error) => {
                    self.files.clear();
                    self.selected_file = None;
                    self.message = format!("Could not read {}: {error}", directory.display());
                }
            },
            Err(error) => {
                self.files.clear();
                self.selected_file = None;
                self.message = error;
            }
        }
    }

    fn find_route(&mut self) {
        self.route = None;
        let Some(index) = self.selected_file else {
            self.message = "Select a portal JSON file first.".to_owned();
            return;
        };
        let graph = match &self.files[index].graph {
            Ok(graph) => graph,
            Err(error) => {
                self.message = format!("Cannot use this file: {error}");
                return;
            }
        };
        let start = match parse_zone(&self.start, "Starting position") {
            Ok(zone) => zone,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        let destination = match parse_zone(&self.destination, "Destination") {
            Ok(zone) => zone,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        if !graph.contains_key(&start) {
            self.message = format!("Starting zone {start} is not in this file.");
            return;
        }
        if !graph.contains_key(&destination) {
            self.message = format!("Destination zone {destination} is not in this file.");
            return;
        }
        match shortest_path(graph, start, destination) {
            Some(route) => {
                self.message = if route.directions.is_empty() {
                    format!("Already at destination zone {destination}.")
                } else {
                    format!(
                        "Shortest route: {} portal(s), {} zone(s).",
                        route.directions.len(),
                        route.zones.len()
                    )
                };
                self.route = Some(route);
            }
            None => {
                self.message = format!("No route from zone {start} to zone {destination}.");
            }
        }
    }

    fn active_graph(&self) -> Option<&BTreeMap<u64, PortalExits>> {
        self.selected_file
            .and_then(|index| self.files.get(index))
            .and_then(|file| file.graph.as_ref().ok())
    }

    fn select_editor_node(&mut self, zone: u64) {
        self.editor_selected = zone;
        let exits = &self.editor_graph[&zone];
        self.editor_targets = [
            exits.west.to_string(),
            exits.north.to_string(),
            exits.east.to_string(),
            exits.south.to_string(),
        ];
    }

    fn apply_editor_targets(&mut self) -> Result<(), String> {
        let mut targets = [0; 4];
        for (index, value) in self.editor_targets.iter().enumerate() {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            let target = value.parse::<u64>().map_err(|_| {
                format!("Target '{}' must be a zone ID from 11 to 55, or 0.", value)
            })?;
            if target == 0 {
                continue;
            }
            if zone_coordinates(target).is_none() {
                return Err(format!(
                    "Target {target} is outside the 5×5 grid; enter 11–55 with X and Y from 1 to 5."
                ));
            }
            targets[index] = target;
        }
        self.editor_graph.insert(
            self.editor_selected,
            PortalExits {
                west: targets[0],
                north: targets[1],
                east: targets[2],
                south: targets[3],
            },
        );
        Ok(())
    }

    fn save_editor_file(&mut self) {
        if let Err(error) = self.apply_editor_targets() {
            self.message = error;
            return;
        }
        let Some(filename) = json_filename(&self.editor_filename) else {
            self.message =
                "Enter a file name using letters, numbers, spaces, '-' or '_'; do not include a path."
                    .to_owned();
            return;
        };
        let directory = match find_portal_directory() {
            Ok(directory) => directory,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        if let Err(error) = fs::create_dir_all(&directory) {
            self.message = format!("Could not create {}: {error}", directory.display());
            return;
        }
        let path = directory.join(&filename);
        let contents = match serde_json::to_vec_pretty(&self.editor_graph) {
            Ok(contents) => contents,
            Err(error) => {
                self.message = format!("Could not encode portal map: {error}");
                return;
            }
        };
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.message = format!(
                    "{} already exists. Choose another name; existing maps are not overwritten.",
                    path.display()
                );
                return;
            }
            Err(error) => {
                self.message = format!("Could not create {}: {error}", path.display());
                return;
            }
        };
        if let Err(error) = file.write_all(&contents) {
            drop(file);
            if let Err(cleanup_error) = fs::remove_file(&path) {
                self.message = format!(
                    "Could not write {}: {error}; also failed to remove the incomplete file: {cleanup_error}",
                    path.display()
                );
            } else {
                self.message = format!("Could not write {}: {error}", path.display());
            }
            return;
        }
        self.reload_files();
        self.selected_file = self
            .files
            .iter()
            .position(|portal_file| portal_file.name == filename);
        self.message = format!("Saved {}.", path.display());
    }
}

impl PortalFile {
    fn load(path: &Path) -> Self {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let graph = fs::read_to_string(path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))
            .and_then(|contents| {
                serde_json::from_str::<BTreeMap<u64, PortalExits>>(&contents)
                    .map_err(|error| format!("Invalid JSON in {}: {error}", path.display()))
            })
            .and_then(|graph| {
                pathfinding::validate_graph(&graph).map_err(|error| {
                    format!("Invalid portal data in {}: {error}", path.display())
                })?;
                Ok(graph)
            });
        Self { name, graph }
    }
}

fn find_portal_directory() -> Result<PathBuf, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("Could not locate the application executable: {error}"))?;
    let adjacent = executable
        .parent()
        .ok_or_else(|| "Could not locate the application directory.".to_owned())?
        .join("portals");
    if adjacent.exists() {
        return Ok(adjacent);
    }
    let development = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("portals");
    if development.exists() {
        return Ok(development);
    }
    Ok(adjacent)
}

fn parse_zone(value: &str, label: &str) -> Result<u64, String> {
    value
        .trim()
        .parse()
        .map_err(|_| format!("{label} must be a numeric zone ID, e.g. 12 for X=1, Y=2."))
}

impl eframe::App for PathfinderApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        let mut visuals = egui::Visuals::dark();
        visuals.override_text_color = Some(Color32::from_rgb(240, 244, 255));
        context.set_visuals(visuals);

        egui::CentralPanel::default().show(context, |ui| {
            ui.heading("Portal Path Finder");
            ui.horizontal(|ui| {
                ui.label("Mode:");
                egui::ComboBox::from_id_salt("app_mode")
                    .selected_text(match self.mode {
                        AppMode::Pathfinder => "Path finder",
                        AppMode::Editor => "Map editor",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.mode,
                            AppMode::Pathfinder,
                            "Path finder",
                        );
                        ui.selectable_value(&mut self.mode, AppMode::Editor, "Map editor");
                    });
            });
            ui.add_space(10.0);

            match self.mode {
                AppMode::Pathfinder => {
                    ui.label("Find the shortest route through directed, non-Euclidean portals.");
                    ui.horizontal(|ui| {
                        ui.label("Portal file:");
                        let selected_name = self
                            .selected_file
                            .and_then(|index| self.files.get(index))
                            .map_or("No files found", |file| file.name.as_str());
                        egui::ComboBox::from_id_salt("portal_file")
                            .selected_text(selected_name)
                            .show_ui(ui, |ui| {
                                for (index, file) in self.files.iter().enumerate() {
                                    let label = if file.graph.is_ok() {
                                        file.name.clone()
                                    } else {
                                        format!("{} (invalid)", file.name)
                                    };
                                    if ui
                                        .selectable_value(
                                            &mut self.selected_file,
                                            Some(index),
                                            label,
                                        )
                                        .changed()
                                    {
                                        self.route = None;
                                    }
                                }
                            });
                        if ui.button("Reload files").clicked() {
                            self.reload_files();
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Starting position (X,Y):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.start)
                                .hint_text("e.g. 12")
                                .desired_width(90.0),
                        );
                        ui.label("Destination (X,Y):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.destination)
                                .hint_text("e.g. 33")
                                .desired_width(90.0),
                        );
                        if ui.button("Find shortest path").clicked() {
                            self.find_route();
                        }
                    });

                    ui.label("Zone IDs encode X then Y (for example, 12 is X=1, Y=2).");
                    ui.label("Click the grid to choose start, then destination; clicks alternate.");
                    ui.label(&self.message);
                    if let Some(route) = &self.route {
                        ui.horizontal_wrapped(|ui| {
                            ui.strong("Directions:");
                            if route.directions.is_empty() {
                                ui.label("Already there");
                            } else {
                                for (index, direction) in route.directions.iter().enumerate() {
                                    if index > 0 {
                                        ui.label(",");
                                    }
                                    ui.label(direction.to_string());
                                }
                            }
                        });
                        ui.label(format!(
                            "Zones: {}",
                            route
                                .zones
                                .iter()
                                .map(u64::to_string)
                                .collect::<Vec<_>>()
                                .join(" → ")
                        ));
                    }

                    ui.separator();
                    ui.label("5×5 coordinate grid (portal destinations may be non-adjacent):");
                    let clicked_zone = if let Some(graph) = self.active_graph() {
                        draw_topology(
                            ui,
                            graph,
                            self.route.as_ref(),
                            &self.start,
                            &self.destination,
                        )
                    } else {
                        ui.label("Choose a valid portal file to display its topology.");
                        None
                    };
                    if let Some(zone) = clicked_zone {
                        self.route = None;
                        if self.next_click_is_start {
                            self.start = zone.to_string();
                            self.destination.clear();
                            self.message = format!(
                                "Start set to zone {zone}. Click the grid again to choose a destination."
                            );
                        } else {
                            self.destination = zone.to_string();
                            self.message = format!(
                                "Destination set to zone {zone}. Choose Find shortest path to calculate the route."
                            );
                        }
                        self.next_click_is_start = !self.next_click_is_start;
                    }

                    if let Some(file) = self
                        .selected_file
                        .and_then(|index| self.files.get(index))
                        .filter(|file| file.graph.is_err())
                    {
                        if let Err(error) = &file.graph {
                            ui.colored_label(Color32::from_rgb(255, 125, 125), error);
                        }
                    }
                }
                AppMode::Editor => {
                    ui.label("Select a node, enter its portal destination zone IDs, then save a new map.");
                    ui.horizontal(|ui| {
                        ui.label("Selected node:");
                        ui.strong(self.editor_selected.to_string());
                        if ui.button("New empty map").clicked() {
                            self.editor_graph = empty_map();
                            self.editor_filename.clear();
                            self.editor_targets = std::array::from_fn(|_| "0".to_owned());
                            self.message = "Started a new empty 5×5 map.".to_owned();
                        }
                    });

                    let clicked_node = draw_editor_grid(ui, &self.editor_graph, self.editor_selected);
                    if let Some(zone) = clicked_node {
                        self.select_editor_node(zone);
                    }

                    ui.label(format!("Targets from node {} (0 means no portal):", self.editor_selected));
                    let mut apply_targets = false;
                    ui.horizontal(|ui| {
                        ui.label("West");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.editor_targets[0])
                                .desired_width(56.0)
                                .hint_text("0"),
                        );
                        ui.label("North");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.editor_targets[1])
                                .desired_width(56.0)
                                .hint_text("0"),
                        );
                        ui.label("East");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.editor_targets[2])
                                .desired_width(56.0)
                                .hint_text("0"),
                        );
                        ui.label("South");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.editor_targets[3])
                                .desired_width(56.0)
                                .hint_text("0"),
                        );
                        apply_targets = ui.button("Apply targets").clicked();
                    });
                    if apply_targets {
                        self.message = match self.apply_editor_targets() {
                            Ok(()) => "Targets applied.".to_owned(),
                            Err(error) => error,
                        };
                    }

                    ui.horizontal(|ui| {
                        ui.label("Save as:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.editor_filename)
                                .desired_width(220.0)
                                .hint_text("my-map.json"),
                        );
                        if ui.button("Create JSON file").clicked() {
                            self.save_editor_file();
                        }
                    });

                    ui.label("Targets must be zone IDs from 11 to 55; blank or 0 means no portal.");
                    ui.label(&self.message);
                }
            }
        });
    }
}

fn zone_coordinates(zone: u64) -> Option<(usize, usize)> {
    let x = zone / 10;
    let y = zone % 10;
    if (1..=GRID_SIZE as u64).contains(&x) && (1..=GRID_SIZE as u64).contains(&y) {
        Some((x as usize, y as usize))
    } else {
        None
    }
}

fn empty_map() -> BTreeMap<u64, PortalExits> {
    (1..=GRID_SIZE)
        .flat_map(|x| (1..=GRID_SIZE).map(move |y| (x * 10 + y) as u64))
        .map(|zone| (zone, PortalExits::default()))
        .collect()
}

fn json_filename(input: &str) -> Option<String> {
    let input = input.trim();
    let stem = if input.to_ascii_lowercase().ends_with(".json") {
        input.get(..input.len() - ".json".len())?
    } else {
        input
    };
    let stem = stem.trim();
    if stem.is_empty()
        || !stem
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, ' ' | '-' | '_'))
    {
        return None;
    }
    Some(format!("{stem}.json"))
}

fn draw_editor_grid(
    ui: &mut egui::Ui,
    graph: &BTreeMap<u64, PortalExits>,
    selected: u64,
) -> Option<u64> {
    let area_width = ui.available_width().max(320.0).min(760.0);
    let area_size = Vec2::new(area_width, 400.0);
    let (rect, response) = ui.allocate_exact_size(area_size, Sense::click());
    let painter = ui.painter_at(rect);
    let grid_rect = egui::Rect::from_min_max(
        Pos2::new(rect.left() + 36.0, rect.top() + 22.0),
        Pos2::new(rect.right() - 12.0, rect.bottom() - 22.0),
    );
    let cell_width = grid_rect.width() / GRID_SIZE as f32;
    let cell_height = grid_rect.height() / GRID_SIZE as f32;
    let position = |zone: u64| -> Option<Pos2> {
        let (x, y) = zone_coordinates(zone)?;
        Some(Pos2::new(
            grid_rect.left() + (x as f32 - 0.5) * cell_width,
            grid_rect.top() + (y as f32 - 0.5) * cell_height,
        ))
    };

    let grid_color = Color32::from_gray(215);
    for index in 0..=GRID_SIZE {
        let x = grid_rect.left() + index as f32 * cell_width;
        let y = grid_rect.top() + index as f32 * cell_height;
        painter.line_segment(
            [
                Pos2::new(x, grid_rect.top()),
                Pos2::new(x, grid_rect.bottom()),
            ],
            Stroke::new(1.0_f32, grid_color),
        );
        painter.line_segment(
            [
                Pos2::new(grid_rect.left(), y),
                Pos2::new(grid_rect.right(), y),
            ],
            Stroke::new(1.0_f32, grid_color),
        );
    }
    for coordinate in 1..=GRID_SIZE {
        painter.text(
            Pos2::new(
                grid_rect.left() + (coordinate as f32 - 0.5) * cell_width,
                rect.top() + 10.0,
            ),
            Align2::CENTER_CENTER,
            coordinate.to_string(),
            FontId::proportional(12.0),
            Color32::from_rgb(190, 200, 218),
        );
        painter.text(
            Pos2::new(
                rect.left() + 18.0,
                grid_rect.top() + (coordinate as f32 - 0.5) * cell_height,
            ),
            Align2::CENTER_CENTER,
            coordinate.to_string(),
            FontId::proportional(12.0),
            Color32::from_rgb(190, 200, 218),
        );
    }

    for (source, exits) in graph {
        let Some(source_position) = position(*source) else {
            continue;
        };
        for (direction, destination) in exits.iter() {
            let Some(destination_position) = position(destination) else {
                continue;
            };
            draw_edge(
                &painter,
                source_position,
                destination_position,
                direction,
                false,
            );
        }
    }

    for zone in graph.keys() {
        let Some(node_position) = position(*zone) else {
            continue;
        };
        let color = if *zone == selected {
            Color32::from_rgb(235, 145, 35)
        } else if graph[zone].iter().next().is_some() {
            Color32::from_rgb(65, 125, 205)
        } else {
            Color32::from_rgb(105, 115, 130)
        };
        painter.circle_filled(node_position, 17.0, color);
        painter.text(
            node_position,
            Align2::CENTER_CENTER,
            zone.to_string(),
            FontId::proportional(12.0),
            Color32::WHITE,
        );
    }

    if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
        && grid_rect.contains(pointer)
    {
        let x = ((pointer.x - grid_rect.left()) / cell_width).floor() as u64 + 1;
        let y = ((pointer.y - grid_rect.top()) / cell_height).floor() as u64 + 1;
        return Some(x * 10 + y);
    }
    None
}

fn draw_topology(
    ui: &mut egui::Ui,
    graph: &BTreeMap<u64, PortalExits>,
    route: Option<&Route>,
    start_text: &str,
    destination_text: &str,
) -> Option<u64> {
    let area_width = ui.available_width().max(320.0).min(760.0);
    let area_size = Vec2::new(area_width, 460.0);
    let (rect, response) = ui.allocate_exact_size(area_size, Sense::click());
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let painter = ui.painter_at(rect);
    let left_padding = 36.0;
    let top_padding = 22.0;
    let right_padding = 12.0;
    let bottom_padding = 22.0;
    let grid_rect = egui::Rect::from_min_max(
        Pos2::new(rect.left() + left_padding, rect.top() + top_padding),
        Pos2::new(rect.right() - right_padding, rect.bottom() - bottom_padding),
    );
    let cell_width = grid_rect.width() / GRID_SIZE as f32;
    let cell_height = grid_rect.height() / GRID_SIZE as f32;
    let position = |zone: u64| -> Option<Pos2> {
        let (x, y) = zone_coordinates(zone)?;
        Some(Pos2::new(
            grid_rect.left() + (x as f32 - 0.5) * cell_width,
            grid_rect.top() + (y as f32 - 0.5) * cell_height,
        ))
    };

    let grid_color = Color32::from_gray(215);
    for index in 0..=GRID_SIZE {
        let x = grid_rect.left() + index as f32 * cell_width;
        let y = grid_rect.top() + index as f32 * cell_height;
        painter.line_segment(
            [
                Pos2::new(x, grid_rect.top()),
                Pos2::new(x, grid_rect.bottom()),
            ],
            Stroke::new(1.0_f32, grid_color),
        );
        painter.line_segment(
            [
                Pos2::new(grid_rect.left(), y),
                Pos2::new(grid_rect.right(), y),
            ],
            Stroke::new(1.0_f32, grid_color),
        );
    }
    for coordinate in 1..=GRID_SIZE {
        painter.text(
            Pos2::new(
                grid_rect.left() + (coordinate as f32 - 0.5) * cell_width,
                rect.top() + 10.0,
            ),
            Align2::CENTER_CENTER,
            coordinate.to_string(),
            FontId::proportional(12.0),
            Color32::from_rgb(190, 200, 218),
        );
        painter.text(
            Pos2::new(
                rect.left() + 18.0,
                grid_rect.top() + (coordinate as f32 - 0.5) * cell_height,
            ),
            Align2::CENTER_CENTER,
            coordinate.to_string(),
            FontId::proportional(12.0),
            Color32::from_rgb(190, 200, 218),
        );
    }

    for (source, exits) in graph {
        let Some(source_position) = position(*source) else {
            continue;
        };
        for (direction, destination) in exits.iter() {
            let Some(destination_position) = position(destination) else {
                continue;
            };
            let highlighted = route.is_some_and(|route| {
                route
                    .zones
                    .windows(2)
                    .zip(&route.directions)
                    .any(|(zones, route_direction)| {
                        zones[0] == *source
                            && zones[1] == destination
                            && *route_direction == direction
                    })
            });
            if !highlighted {
                continue;
            }
            draw_edge(
                &painter,
                source_position,
                destination_position,
                direction,
                highlighted,
            );
        }
    }

    let start = parse_zone(start_text, "").ok();
    let destination = parse_zone(destination_text, "").ok();
    for zone in graph.keys() {
        let Some(position) = position(*zone) else {
            continue;
        };
        let color = if Some(*zone) == start {
            Color32::from_rgb(55, 160, 95)
        } else if Some(*zone) == destination {
            Color32::from_rgb(205, 75, 75)
        } else if route.is_some_and(|route| route.zones.contains(zone)) {
            Color32::from_rgb(65, 125, 205)
        } else {
            Color32::from_rgb(105, 115, 130)
        };
        painter.circle_filled(position, 17.0, color);
        painter.text(
            position,
            Align2::CENTER_CENTER,
            zone.to_string(),
            FontId::proportional(12.0),
            Color32::WHITE,
        );
    }

    if graph.keys().any(|zone| zone_coordinates(*zone).is_none()) {
        ui.colored_label(
            Color32::from_rgb(255, 185, 95),
            "Zones outside coordinates X=1..5, Y=1..5 are omitted from the grid.",
        );
    }

    if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
        && grid_rect.contains(pointer)
    {
        let x = ((pointer.x - grid_rect.left()) / cell_width).floor() as u64 + 1;
        let y = ((pointer.y - grid_rect.top()) / cell_height).floor() as u64 + 1;
        return Some(x * 10 + y);
    }
    None
}

fn draw_edge(
    painter: &egui::Painter,
    source: Pos2,
    destination: Pos2,
    direction: Direction,
    highlighted: bool,
) {
    let delta = destination - source;
    let length = delta.length();
    if length < 36.0 {
        return;
    }
    let unit = delta / length;
    let color = if highlighted {
        Color32::from_rgb(235, 145, 35)
    } else {
        Color32::from_gray(145)
    };
    let stroke = Stroke::new(if highlighted { 3.0_f32 } else { 1.3_f32 }, color);
    let start = source + unit * 18.0;
    let end = destination - unit * 19.0;
    painter.line_segment([start, end], stroke);

    let perpendicular = Vec2::new(-unit.y, unit.x);
    painter.add(egui::Shape::convex_polygon(
        vec![
            end,
            end - unit * 8.0 + perpendicular * 4.0,
            end - unit * 8.0 - perpendicular * 4.0,
        ],
        color,
        Stroke::NONE,
    ));
    let label_position = Pos2::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0);
    painter.rect_filled(
        egui::Rect::from_center_size(label_position, Vec2::new(20.0, 18.0)),
        4.0,
        Color32::from_rgb(25, 35, 75),
    );
    painter.text(
        label_position,
        Align2::CENTER_CENTER,
        direction.short_name(),
        FontId::proportional(12.0),
        Color32::WHITE,
    );
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1050.0, 760.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Portal Path Finder",
        options,
        Box::new(|_creation_context| Ok(Box::new(PathfinderApp::new()))),
    )
}

#[cfg(test)]
mod tests {
    use super::{empty_map, json_filename, zone_coordinates};

    #[test]
    fn zone_ids_map_to_x_then_y_coordinates() {
        assert_eq!(zone_coordinates(12), Some((1, 2)));
        assert_eq!(zone_coordinates(55), Some((5, 5)));
        assert_eq!(zone_coordinates(60), None);
    }

    #[test]
    fn editor_starts_with_all_twenty_five_grid_nodes() {
        assert_eq!(empty_map().len(), 25);
    }

    #[test]
    fn editor_map_serializes_to_the_portal_json_shape() {
        let mut graph = empty_map();
        graph.get_mut(&12).unwrap().north = 22;
        let json = serde_json::to_value(graph).unwrap();
        assert_eq!(json["12"]["north"], 22);
        assert_eq!(json["12"]["west"], 0);
    }

    #[test]
    fn json_filename_adds_extension_and_rejects_paths() {
        assert_eq!(json_filename("my-map"), Some("my-map.json".to_owned()));
        assert_eq!(json_filename("my-map.JSON"), Some("my-map.json".to_owned()));
        assert_eq!(json_filename("../map"), None);
        assert_eq!(json_filename(""), None);
    }
}
