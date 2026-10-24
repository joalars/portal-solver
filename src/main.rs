mod map;
mod map_migrations;
mod pathfinding;
mod updater;

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Sense, Stroke, Vec2};
use map::{MapLayout, PortalMap};
use pathfinding::{Direction, PortalExits, Route, shortest_path};

struct PortalFile {
    name: String,
    map: Result<PortalMap, String>,
}

enum GridInteraction {
    Click(u64),
    DragStarted(u64),
    DragEnded(Option<u64>),
}

enum EditorGridInteraction {
    NodeClicked(u64),
    HandleClicked(u64, Direction),
    HandleDragStarted(u64, Direction),
    HandleDragEnded(Option<u64>),
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
    drag_start: Option<u64>,
    editor_map: PortalMap,
    editor_selected: u64,
    editor_cell_id: String,
    editor_targets: [String; 4],
    editor_pending_target: Option<(u64, Direction)>,
    editor_drag: Option<(u64, Direction)>,
    editor_filename: String,
    editor_file_selection: Option<usize>,
    editor_loaded_file: Option<String>,
    editor_width: usize,
    editor_height: usize,
    update_sender: Sender<UpdateEvent>,
    update_receiver: Receiver<UpdateEvent>,
    update_busy: bool,
    available_update: Option<updater::Release>,
    update_status: String,
    confirm_update: bool,
}

enum UpdateEvent {
    Checked(Result<Option<updater::Release>, String>),
    Installed(Result<(), String>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Pathfinder,
    Editor,
}

impl PathfinderApp {
    fn new() -> Self {
        let (update_sender, update_receiver) = mpsc::channel();
        let mut app = Self {
            files: Vec::new(),
            selected_file: None,
            start: String::new(),
            destination: String::new(),
            route: None,
            message: String::new(),
            mode: AppMode::Pathfinder,
            next_click_is_start: true,
            drag_start: None,
            editor_map: PortalMap::new_grid(5, 5).expect("default grid dimensions are valid"),
            editor_selected: 11,
            editor_cell_id: "11".to_owned(),
            editor_targets: std::array::from_fn(|_| "0".to_owned()),
            editor_pending_target: None,
            editor_drag: None,
            editor_filename: String::new(),
            editor_file_selection: None,
            editor_loaded_file: None,
            editor_width: 5,
            editor_height: 5,
            update_sender,
            update_receiver,
            update_busy: false,
            available_update: None,
            update_status: String::new(),
            confirm_update: false,
        };
        app.reload_files();
        app.check_for_updates();
        app
    }

    fn check_for_updates(&mut self) {
        self.update_busy = true;
        self.available_update = None;
        self.update_status = "Checking GitHub for updates…".to_owned();
        let sender = self.update_sender.clone();
        thread::spawn(move || {
            let _ = sender.send(UpdateEvent::Checked(updater::check_for_update()));
        });
    }

    fn install_update(&mut self) {
        let Some(release) = self.available_update.clone() else {
            self.update_status = "Check for an update before installing.".to_owned();
            return;
        };
        self.update_busy = true;
        self.update_status = format!("Downloading version {}…", release.version);
        let sender = self.update_sender.clone();
        thread::spawn(move || {
            let _ = sender.send(UpdateEvent::Installed(updater::install_and_restart(
                &release,
            )));
        });
    }

    fn poll_update_events(&mut self, context: &egui::Context) {
        while let Ok(event) = self.update_receiver.try_recv() {
            match event {
                UpdateEvent::Checked(Ok(Some(release))) => {
                    self.update_status.clear();
                    self.available_update = Some(release);
                    self.update_busy = false;
                }
                UpdateEvent::Checked(Ok(None)) => {
                    self.available_update = None;
                    self.update_status.clear();
                    self.update_busy = false;
                }
                UpdateEvent::Checked(Err(error)) => {
                    self.available_update = None;
                    self.update_status = error;
                    self.update_busy = false;
                }
                UpdateEvent::Installed(Ok(())) => {
                    self.update_status = "Update installed; restarting…".to_owned();
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                UpdateEvent::Installed(Err(error)) => {
                    self.update_status = error;
                    self.update_busy = false;
                }
            }
        }
        if self.update_busy {
            context.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn reload_files(&mut self) {
        self.clear_path_selection();
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

    fn clear_path_selection(&mut self) {
        self.route = None;
        self.start.clear();
        self.destination.clear();
        self.next_click_is_start = true;
        self.drag_start = None;
        self.message.clear();
    }

    fn find_route(&mut self) {
        self.route = None;
        let Some(index) = self.selected_file else {
            self.message = "Select a portal JSON file first.".to_owned();
            return;
        };
        let map = match &self.files[index].map {
            Ok(map) => map,
            Err(error) => {
                self.message = format!("Cannot use this file: {error}");
                return;
            }
        };
        let graph = map.graph();
        let start = match parse_cell_id(&self.start, "Starting cell") {
            Ok(zone) => zone,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        let destination = match parse_cell_id(&self.destination, "Destination") {
            Ok(zone) => zone,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        if !graph.contains_key(&start) {
            self.message = format!("Starting cell {start} is not in this file.");
            return;
        }
        if !graph.contains_key(&destination) {
            self.message = format!("Destination cell {destination} is not in this file.");
            return;
        }
        match shortest_path(&graph, start, destination) {
            Some(route) => {
                self.message = if route.directions.is_empty() {
                    format!("Already at destination cell {destination}.")
                } else {
                    format!(
                        "Shortest route: {} portal(s), {} cell(s).",
                        route.directions.len(),
                        route.zones.len()
                    )
                };
                self.route = Some(route);
            }
            None => {
                self.message = format!("No route from cell {start} to cell {destination}.");
            }
        }
    }

    fn active_map(&self) -> Option<&PortalMap> {
        self.selected_file
            .and_then(|index| self.files.get(index))
            .and_then(|file| file.map.as_ref().ok())
    }

    fn select_editor_node(&mut self, zone: u64) {
        self.editor_selected = zone;
        self.editor_cell_id = zone.to_string();
        let exits = &self.editor_map.cells[&zone].exits;
        self.editor_targets = [
            exits.west.to_string(),
            exits.north.to_string(),
            exits.east.to_string(),
            exits.south.to_string(),
        ];
    }

    fn set_editor_target(&mut self, source: u64, direction: Direction, target: u64) {
        let Some(cell) = self.editor_map.cells.get_mut(&source) else {
            self.message = format!("Cell {source} no longer exists.");
            return;
        };
        let exits = &mut cell.exits;
        let current_target = match direction {
            Direction::West => &mut exits.west,
            Direction::North => &mut exits.north,
            Direction::East => &mut exits.east,
            Direction::South => &mut exits.south,
        };
        let (updated_target, cleared) = toggled_target(*current_target, target);
        *current_target = updated_target;
        if source == self.editor_selected {
            self.select_editor_node(source);
        }
        self.message = if cleared {
            format!("{direction} portal from {source} cleared.")
        } else {
            format!("{direction} portal from {source} set to {target}.")
        };
    }

    fn rename_selected_graph_cell(&mut self) {
        let old_id = self.editor_selected;
        let new_id = match self.editor_cell_id.trim().parse::<u64>() {
            Ok(id) => id,
            Err(_) => {
                self.message = "Cell ID must be a positive number.".to_owned();
                return;
            }
        };
        match self
            .editor_map
            .rename_graph_cell(self.editor_selected, new_id)
        {
            Ok(()) => {
                self.select_editor_node(new_id);
                self.message =
                    format!("Renamed cell {old_id} to {new_id}; portal references were updated.");
            }
            Err(error) => self.message = error,
        }
    }

    fn load_editor_file(&mut self) {
        let Some(index) = self.editor_file_selection else {
            self.message = "Select a valid portal map to load.".to_owned();
            return;
        };
        let Some(file) = self.files.get(index) else {
            self.message =
                "The selected portal map is no longer available; reload files.".to_owned();
            return;
        };
        let name = file.name.clone();
        let mut map = match &file.map {
            Ok(map) => map.clone(),
            Err(error) => {
                self.message = format!("Cannot edit this file: {error}");
                return;
            }
        };
        if let Some((width, height)) = map.grid_dimensions() {
            if let Err(error) = map.resize_grid(width, height) {
                self.message = error;
                return;
            }
            self.editor_width = width;
            self.editor_height = height;
        }
        self.editor_map = map;
        self.editor_selected = self.editor_map.cells.keys().next().copied().unwrap_or(0);
        if self.editor_selected == 0 && matches!(&self.editor_map.layout, MapLayout::Graph) {
            if let Err(error) = self.editor_map.add_graph_cell() {
                self.message = error;
                return;
            }
            self.editor_selected = 11;
        }
        self.select_editor_node(self.editor_selected);
        self.editor_filename = name.clone();
        self.editor_loaded_file = Some(name.clone());
        self.message = format!("Loaded {name} for editing.");
    }

    fn apply_editor_targets(&mut self) -> Result<(), String> {
        let mut targets = [0; 4];
        for (index, value) in self.editor_targets.iter().enumerate() {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            let target = value
                .parse::<u64>()
                .map_err(|_| format!("Target '{}' must be a numeric cell ID, or 0.", value))?;
            if target == 0 {
                continue;
            }
            if !self.editor_map.cells.contains_key(&target) {
                return Err(format!("Target cell {target} does not exist in this map."));
            }
            targets[index] = target;
        }
        let Some(cell) = self.editor_map.cells.get_mut(&self.editor_selected) else {
            return Err("Select an existing cell before editing its portals.".to_owned());
        };
        cell.exits = PortalExits {
            west: targets[0],
            north: targets[1],
            east: targets[2],
            south: targets[3],
        };
        Ok(())
    }

    fn save_editor_changes(&mut self) {
        let Some(filename) = self.editor_loaded_file.clone() else {
            self.message = "Load an existing map before saving changes.".to_owned();
            return;
        };
        if let Err(error) = self.apply_editor_targets() {
            self.message = error;
            return;
        }
        if let Err(error) = self.editor_map.validate() {
            self.message = error;
            return;
        }
        let directory = match find_portal_directory() {
            Ok(directory) => directory,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        let path = directory.join(&filename);
        let contents = match serde_json::to_vec_pretty(&self.editor_map) {
            Ok(contents) => contents,
            Err(error) => {
                self.message = format!("Could not encode portal map: {error}");
                return;
            }
        };
        match write_atomically(&path, &contents) {
            Ok(()) => {
                self.reload_files();
                self.editor_file_selection =
                    self.files.iter().position(|file| file.name == filename);
                self.selected_file = self.editor_file_selection;
                self.message = format!("Saved changes to {}.", path.display());
            }
            Err(error) => self.message = format!("Could not save {}: {error}", path.display()),
        }
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
        let path = directory.join(&filename);
        if let Err(error) = self.editor_map.validate() {
            self.message = error;
            return;
        }
        let contents = match serde_json::to_vec_pretty(&self.editor_map) {
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
        self.editor_file_selection = self.selected_file;
        self.editor_loaded_file = Some(filename.clone());
        self.editor_filename = filename.clone();
        self.message = format!("Saved {}.", path.display());
    }
}

impl PortalFile {
    fn load(path: &Path) -> Self {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let map = fs::read_to_string(path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))
            .and_then(|contents| {
                PortalMap::from_json(&contents)
                    .map_err(|error| format!("Invalid portal data in {}: {error}", path.display()))
            });
        Self { name, map }
    }
}

fn find_portal_directory() -> Result<PathBuf, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("Could not locate the application executable: {error}"))?;
    let directory = executable
        .parent()
        .ok_or_else(|| "Could not locate the application directory.".to_owned())?
        .join("portals");
    ensure_portal_directory(&directory)
}

fn ensure_portal_directory(directory: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("Could not create {}: {error}", directory.display()))?;
    let example_path = directory.join("example.json");
    let mut example = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&example_path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Ok(directory.to_path_buf());
        }
        Err(error) => {
            return Err(format!(
                "Could not create default map {}: {error}",
                example_path.display()
            ));
        }
    };
    if let Err(error) = example.write_all(include_bytes!("../portals/example.json")) {
        drop(example);
        if let Err(cleanup_error) = fs::remove_file(&example_path) {
            return Err(format!(
                "Could not write default map {}: {error}; also failed to remove the incomplete file: {cleanup_error}",
                example_path.display()
            ));
        }
        return Err(format!(
            "Could not write default map {}: {error}",
            example_path.display()
        ));
    }
    Ok(directory.to_path_buf())
}

fn parse_cell_id(value: &str, label: &str) -> Result<u64, String> {
    value
        .trim()
        .parse()
        .map_err(|_| format!("{label} must be a numeric cell ID."))
}

impl eframe::App for PathfinderApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_update_events(context);
        let mut visuals = egui::Visuals::dark();
        visuals.override_text_color = Some(Color32::from_rgb(240, 244, 255));
        context.set_visuals(visuals);

        egui::CentralPanel::default().show(context, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Portal Path Finder");
                ui.label(format!("v{}", env!("CARGO_PKG_VERSION")));
            });
            ui.horizontal(|ui| {
                if let Some(release) = &self.available_update {
                    let label = format!("Install {} and restart", release.version);
                    if ui
                        .add_enabled(!self.update_busy, egui::Button::new(label))
                        .clicked()
                    {
                        self.confirm_update = true;
                    }
                }
                if !self.update_status.is_empty() {
                    ui.label(&self.update_status);
                }
            });
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
                    let interaction = if let Some(map) = self.active_map() {
                        draw_topology(
                            ui,
                            map,
                            self.route.as_ref(),
                            &self.start,
                            &self.destination,
                            self.drag_start,
                        )
                    } else {
                        ui.allocate_space(Vec2::new(ui.available_width(), 460.0));
                        None
                    };
                    match interaction {
                        Some(GridInteraction::Click(zone)) => {
                            self.drag_start = None;
                            self.route = None;
                            if self.next_click_is_start {
                                self.start = zone.to_string();
                                self.destination.clear();
                                self.message = format!(
                                    "Start set to cell {zone}. Click again or drag to choose a destination."
                                );
                                self.next_click_is_start = false;
                            } else {
                                self.destination = zone.to_string();
                                self.message = format!("Destination set to cell {zone}.");
                                self.next_click_is_start = true;
                                self.find_route();
                            }
                        }
                        Some(GridInteraction::DragStarted(zone)) => {
                            self.drag_start = Some(zone);
                            self.start = zone.to_string();
                            self.destination.clear();
                            self.route = None;
                            self.next_click_is_start = false;
                            self.message = format!("Dragging from cell {zone}…");
                        }
                        Some(GridInteraction::DragEnded(Some(zone))) => {
                            if let Some(start) = self.drag_start.take() {
                                self.start = start.to_string();
                                self.destination = zone.to_string();
                                self.next_click_is_start = true;
                                self.message = format!(
                                    "Route from cell {start} to cell {zone}."
                                );
                                self.find_route();
                            }
                        }
                        Some(GridInteraction::DragEnded(None)) => {
                            self.drag_start = None;
                            self.next_click_is_start = true;
                        }
                        None => {}
                    }

                    ui.separator();
                    if let Some(map) = self.active_map() {
                        if let Some((width, height)) = map.grid_dimensions() {
                            ui.label(format!(
                                "{width}×{height} grid (portal destinations may be non-adjacent):"
                            ));
                        } else {
                            ui.label("Ungridged map; cells are arranged automatically.");
                        }
                    }
                    if self.active_map().is_none() {
                        ui.label("Choose a valid portal file to display its topology.");
                    }
                    ui.label("Find the shortest route through directed, non-Euclidean portals.");
                    ui.horizontal(|ui| {
                        ui.label("Portal file:");
                        let selected_name = self
                            .selected_file
                            .and_then(|index| self.files.get(index))
                            .map_or("No files found", |file| file.name.as_str());
                        let mut file_changed = false;
                        egui::ComboBox::from_id_salt("portal_file")
                            .selected_text(selected_name)
                            .show_ui(ui, |ui| {
                                for (index, file) in self.files.iter().enumerate() {
                                    let label = if file.map.is_ok() {
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
                                        file_changed = true;
                                    }
                                }
                            });
                        if file_changed {
                            self.clear_path_selection();
                        }
                        if ui.button("Reload files").clicked() {
                            self.reload_files();
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Starting cell ID:");
                        let start_response = ui.add(
                            egui::TextEdit::singleline(&mut self.start)
                                .hint_text("e.g. 12")
                                .desired_width(90.0),
                        );
                        ui.label("Destination cell ID:");
                        let destination_response = ui.add(
                            egui::TextEdit::singleline(&mut self.destination)
                                .hint_text("e.g. 33")
                                .desired_width(90.0),
                        );
                        let submit_on_enter = should_submit_on_enter(&start_response, ui)
                            || should_submit_on_enter(&destination_response, ui);
                        if ui.button("Find shortest path").clicked() || submit_on_enter {
                            self.find_route();
                        }
                    });

                    ui.label("Enter numeric cell IDs; legacy 5×5 maps retain their original IDs.");
                    ui.label("Click twice to choose start and destination, or drag from start to destination.");
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
                            "Cells: {}",
                            route
                                .zones
                                .iter()
                                .map(u64::to_string)
                                .collect::<Vec<_>>()
                                .join(" → ")
                        ));
                    }

                    if let Some(file) = self
                        .selected_file
                        .and_then(|index| self.files.get(index))
                        .filter(|file| file.map.is_err())
                    {
                        if let Err(error) = &file.map {
                            ui.colored_label(Color32::from_rgb(255, 125, 125), error);
                        }
                    }
                }
                AppMode::Editor => {
                    let interaction = draw_editor_grid(
                        ui,
                        &self.editor_map,
                        self.editor_selected,
                        self.editor_drag,
                    );
                    match interaction {
                        Some(EditorGridInteraction::NodeClicked(zone)) => {
                            if let Some((source, direction)) = self.editor_pending_target.take() {
                                self.set_editor_target(source, direction, zone);
                            } else {
                                self.select_editor_node(zone);
                            }
                        }
                        Some(EditorGridInteraction::HandleClicked(source, direction)) => {
                            self.select_editor_node(source);
                            self.editor_pending_target = Some((source, direction));
                            self.message = format!(
                                "Click a destination tile for the {direction} portal from {source}, or drag the handle there."
                            );
                        }
                        Some(EditorGridInteraction::HandleDragStarted(source, direction)) => {
                            self.select_editor_node(source);
                            self.editor_pending_target = None;
                            self.editor_drag = Some((source, direction));
                            self.message =
                                format!("Dragging the {direction} portal from {source}…");
                        }
                        Some(EditorGridInteraction::HandleDragEnded(Some(target))) => {
                            if let Some((source, direction)) = self.editor_drag.take() {
                                self.set_editor_target(source, direction, target);
                            }
                        }
                        Some(EditorGridInteraction::HandleDragEnded(None)) => {
                            self.editor_drag = None;
                        }
                        None => {}
                    }

                    ui.separator();
                    if let Some((width, height)) = self.editor_map.grid_dimensions() {
                        ui.label(format!(
                            "{width}×{height} map grid; click a cell to configure its portals."
                        ));
                    } else {
                        ui.label("Ungridged map; cells are arranged automatically.");
                    }
                    ui.label("Select a cell, configure portals with its edge handles or target fields, then save.");
                    let selected_editor_name = self
                        .editor_file_selection
                        .and_then(|index| self.files.get(index))
                        .map_or_else(|| "Select existing map".to_owned(), |file| file.name.clone());
                    let mut load_editor_map = false;
                    ui.horizontal(|ui| {
                        ui.label("Existing map:");
                        egui::ComboBox::from_id_salt("editor_file")
                            .selected_text(selected_editor_name)
                            .show_ui(ui, |ui| {
                                for (index, file) in self.files.iter().enumerate() {
                                    let label = if file.map.is_ok() {
                                        file.name.clone()
                                    } else {
                                        format!("{} (invalid)", file.name)
                                    };
                                    ui.selectable_value(
                                        &mut self.editor_file_selection,
                                        Some(index),
                                        label,
                                    );
                                }
                            });
                        load_editor_map = ui.button("Load for editing").clicked();
                        if ui.button("Reload files").clicked() {
                            self.reload_files();
                            self.editor_file_selection = self
                                .editor_loaded_file
                                .as_ref()
                                .and_then(|name| self.files.iter().position(|file| &file.name == name));
                        }
                    });
                    if load_editor_map {
                        self.load_editor_file();
                    }
                    ui.horizontal(|ui| {
                        ui.label("Selected cell:");
                        ui.strong(self.editor_selected.to_string());
                        if ui.button("New 5×5 grid").clicked() {
                            self.editor_map =
                                PortalMap::new_grid(5, 5).expect("default grid dimensions are valid");
                            self.editor_selected = 11;
                            self.editor_width = 5;
                            self.editor_height = 5;
                            self.editor_filename.clear();
                            self.editor_file_selection = None;
                            self.editor_loaded_file = None;
                            self.editor_pending_target = None;
                            self.editor_drag = None;
                            self.select_editor_node(self.editor_selected);
                            self.message = "Started a new 5×5 grid map.".to_owned();
                        }
                        if ui.button("New ungridded map").clicked() {
                            self.editor_map = PortalMap::new_graph();
                            self.editor_selected = 0;
                            self.editor_filename.clear();
                            self.editor_file_selection = None;
                            self.editor_loaded_file = None;
                            self.editor_pending_target = None;
                            self.editor_drag = None;
                            self.editor_targets = std::array::from_fn(|_| "0".to_owned());
                            self.message = "Started a new ungridded map. Add cells to begin.".to_owned();
                        }
                        if matches!(&self.editor_map.layout, MapLayout::Graph)
                            && ui.button("Add cell").clicked()
                        {
                            match self.editor_map.add_graph_cell() {
                                Ok(id) => {
                                    self.select_editor_node(id);
                                    self.message = format!("Added cell {id}.");
                                }
                                Err(error) => self.message = error,
                            }
                        }
                        if matches!(&self.editor_map.layout, MapLayout::Graph)
                            && !self.editor_map.cells.is_empty()
                            && ui.button("Remove selected cell").clicked()
                        {
                            match self.editor_map.remove_cell(self.editor_selected) {
                                Ok(()) => {
                                    self.editor_selected =
                                        self.editor_map.cells.keys().next().copied().unwrap_or(0);
                                    if self.editor_selected != 0 {
                                        self.select_editor_node(self.editor_selected);
                                    }
                                    self.message = "Removed the selected cell and cleared incoming portals.".to_owned();
                                }
                                Err(error) => self.message = error,
                            }
                        }
                    });
                    if matches!(&self.editor_map.layout, MapLayout::Graph)
                        && self.editor_selected != 0
                    {
                        let mut rename_cell = false;
                        ui.horizontal(|ui| {
                            ui.label("Cell ID:");
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut self.editor_cell_id)
                                    .desired_width(100.0),
                            );
                            let enter_pressed = should_submit_on_enter(&response, ui);
                            rename_cell =
                                ui.button("Rename selected cell").clicked() || enter_pressed;
                        });
                        if rename_cell {
                            self.rename_selected_graph_cell();
                        }
                    }
                    if self.editor_map.grid_dimensions().is_some() {
                        let mut resize_grid = false;
                        ui.horizontal(|ui| {
                            ui.label("Grid size:");
                            ui.add(
                                egui::DragValue::new(&mut self.editor_width)
                                    .range(1..=map::MAX_GRID_DIMENSION)
                                    .prefix("Width "),
                            );
                            ui.add(
                                egui::DragValue::new(&mut self.editor_height)
                                    .range(1..=map::MAX_GRID_DIMENSION)
                                    .prefix("Height "),
                            );
                            resize_grid = ui.button("Resize grid").clicked();
                        });
                        if resize_grid {
                            match self.editor_map.resize_grid(self.editor_width, self.editor_height) {
                                Ok(removed) => {
                                    if !self.editor_map.cells.contains_key(&self.editor_selected) {
                                        self.editor_selected =
                                            self.editor_map.cells.keys().next().copied().unwrap_or(0);
                                    }
                                    if self.editor_selected != 0 {
                                        self.select_editor_node(self.editor_selected);
                                    }
                                    self.message = format!(
                                        "Resized grid to {}×{}; removed {removed} existing cell(s) outside the new bounds.",
                                        self.editor_width, self.editor_height
                                    );
                                }
                                Err(error) => self.message = error,
                            }
                        }
                    }

                    ui.label(format!("Targets from node {} (0 means no portal):", self.editor_selected));
                    let mut apply_targets = false;
                    ui.horizontal(|ui| {
                        let mut enter_pressed = false;
                        ui.label("West");
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.editor_targets[0])
                                .desired_width(56.0)
                                .hint_text("0"),
                        );
                        enter_pressed |= should_submit_on_enter(&response, ui);
                        ui.label("North");
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.editor_targets[1])
                                .desired_width(56.0)
                                .hint_text("0"),
                        );
                        enter_pressed |= should_submit_on_enter(&response, ui);
                        ui.label("East");
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.editor_targets[2])
                                .desired_width(56.0)
                                .hint_text("0"),
                        );
                        enter_pressed |= should_submit_on_enter(&response, ui);
                        ui.label("South");
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.editor_targets[3])
                                .desired_width(56.0)
                                .hint_text("0"),
                        );
                        enter_pressed |= should_submit_on_enter(&response, ui);
                        apply_targets = ui.button("Apply targets").clicked() || enter_pressed;
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
                    if let Some(filename) = &self.editor_loaded_file {
                        let mut save_existing = false;
                        ui.horizontal(|ui| {
                            ui.label(format!("Editing {filename}"));
                            save_existing = ui.button("Save changes").clicked();
                        });
                        if save_existing {
                            self.save_editor_changes();
                        }
                    }

                    ui.label("Targets must be cell IDs in this map; blank or 0 means no portal.");
                    ui.label(&self.message);
                }
            }
        });

        if self.confirm_update {
            let version = self.available_update.as_ref().map_or_else(
                || "the available version".to_owned(),
                |release| release.version.clone(),
            );
            let mut dialog_open = self.confirm_update;
            let mut cancel_clicked = false;
            let mut confirmed = false;
            egui::Window::new("Confirm update")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .open(&mut dialog_open)
                .show(context, |ui| {
                    ui.label(format!(
                        "Install version {version} and restart the application?"
                    ));
                    ui.label("The application executable will be replaced.");
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            cancel_clicked = true;
                        }
                        if ui.button("Install and restart").clicked() {
                            confirmed = true;
                        }
                    });
                });
            self.confirm_update = dialog_open && !cancel_clicked && !confirmed;
            if confirmed {
                self.install_update();
            }
        }
    }
}

fn toggled_target(current: u64, target: u64) -> (u64, bool) {
    if current == target {
        (0, true)
    } else {
        (target, false)
    }
}

fn should_submit_on_enter(response: &egui::Response, ui: &egui::Ui) -> bool {
    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
}

fn write_atomically(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_nanos();
    let filename = path
        .file_name()
        .map_or_else(|| "portal-map".into(), |name| name.to_string_lossy());
    let temporary_path = path.with_file_name(format!(
        ".{filename}.{}.{}.tmp",
        std::process::id(),
        timestamp
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary_path, path)
    })();
    if let Err(error) = result {
        if temporary_path.exists()
            && let Err(cleanup_error) = fs::remove_file(&temporary_path)
        {
            return Err(std::io::Error::other(format!(
                "{error}; failed to remove temporary file {}: {cleanup_error}",
                temporary_path.display()
            )));
        }
        return Err(error);
    }
    Ok(())
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

fn grid_cell_size(map: &PortalMap, rect: egui::Rect) -> Option<(f32, f32)> {
    let (width, height) = map.grid_dimensions()?;
    Some((rect.width() / width as f32, rect.height() / height as f32))
}

fn cell_position(map: &PortalMap, id: u64, rect: egui::Rect) -> Option<Pos2> {
    let cell = map.cells.get(&id)?;
    match &map.layout {
        MapLayout::Grid { width, height } => {
            let (x, y) = (cell.x?, cell.y?);
            Some(Pos2::new(
                rect.left() + (x as f32 - 0.5) * rect.width() / *width as f32,
                rect.top() + (y as f32 - 0.5) * rect.height() / *height as f32,
            ))
        }
        MapLayout::Graph => {
            let count = map.cells.len();
            let index = map.cells.keys().position(|cell_id| *cell_id == id)?;
            if count == 1 {
                return Some(rect.center());
            }
            let angle =
                std::f32::consts::TAU * index as f32 / count as f32 - std::f32::consts::FRAC_PI_2;
            let radius = rect.width().min(rect.height()) * 0.38;
            Some(rect.center() + Vec2::new(angle.cos(), angle.sin()) * radius)
        }
    }
}

fn cell_radius(map: &PortalMap, id: u64, rect: egui::Rect) -> Option<f32> {
    map.cells.get(&id)?;
    match &map.layout {
        MapLayout::Grid { .. } => {
            let (cell_width, cell_height) = grid_cell_size(map, rect)?;
            Some((cell_width.min(cell_height) * 0.32).clamp(6.0, 17.0))
        }
        MapLayout::Graph => Some(17.0),
    }
}

fn direction_vector(direction: Direction) -> Vec2 {
    match direction {
        Direction::West => Vec2::new(-1.0, 0.0),
        Direction::North => Vec2::new(0.0, -1.0),
        Direction::East => Vec2::new(1.0, 0.0),
        Direction::South => Vec2::new(0.0, 1.0),
    }
}

fn handle_hit_radius(map: &PortalMap, id: u64, map_rect: egui::Rect) -> f32 {
    if matches!(&map.layout, MapLayout::Grid { .. }) {
        (cell_radius(map, id, map_rect).unwrap_or(17.0) * 0.55).max(7.0) + 4.0
    } else {
        12.0
    }
}

fn cell_at_position(map: &PortalMap, rect: egui::Rect, point: Pos2) -> Option<u64> {
    if !rect.contains(point) {
        return None;
    }
    match &map.layout {
        MapLayout::Grid { width, height } => {
            let x = ((point.x - rect.left()) / rect.width() * *width as f32).floor() as usize + 1;
            let y = ((point.y - rect.top()) / rect.height() * *height as f32).floor() as usize + 1;
            map.cell_at(x, y)
        }
        MapLayout::Graph => map
            .cells
            .keys()
            .filter_map(|id| cell_position(map, *id, rect).map(|center| (*id, center)))
            .min_by(|(_, left), (_, right)| left.distance(point).total_cmp(&right.distance(point)))
            .filter(|(_, center)| center.distance(point) <= 22.0)
            .map(|(id, _)| id),
    }
}

fn draw_map_grid(
    painter: &egui::Painter,
    map: &PortalMap,
    outer_rect: egui::Rect,
    map_rect: egui::Rect,
) {
    let MapLayout::Grid { width, height } = &map.layout else {
        return;
    };
    let grid_color = Color32::from_gray(215);
    for index in 0..=*width {
        let x = map_rect.left() + index as f32 * map_rect.width() / *width as f32;
        painter.line_segment(
            [
                Pos2::new(x, map_rect.top()),
                Pos2::new(x, map_rect.bottom()),
            ],
            Stroke::new(1.0_f32, grid_color),
        );
    }
    for index in 0..=*height {
        let y = map_rect.top() + index as f32 * map_rect.height() / *height as f32;
        painter.line_segment(
            [
                Pos2::new(map_rect.left(), y),
                Pos2::new(map_rect.right(), y),
            ],
            Stroke::new(1.0_f32, grid_color),
        );
    }
    for coordinate in 1..=*width {
        painter.text(
            Pos2::new(
                map_rect.left() + (coordinate as f32 - 0.5) * map_rect.width() / *width as f32,
                outer_rect.top() + 10.0,
            ),
            Align2::CENTER_CENTER,
            coordinate.to_string(),
            FontId::proportional(12.0),
            Color32::from_rgb(190, 200, 218),
        );
    }
    for coordinate in 1..=*height {
        painter.text(
            Pos2::new(
                outer_rect.left() + 18.0,
                map_rect.top() + (coordinate as f32 - 0.5) * map_rect.height() / *height as f32,
            ),
            Align2::CENTER_CENTER,
            coordinate.to_string(),
            FontId::proportional(12.0),
            Color32::from_rgb(190, 200, 218),
        );
    }
}

fn draw_editor_grid(
    ui: &mut egui::Ui,
    map: &PortalMap,
    selected: u64,
    active_drag: Option<(u64, Direction)>,
) -> Option<EditorGridInteraction> {
    let area_width = ui.available_width().max(320.0).min(760.0);
    let area_size = Vec2::new(area_width, 400.0);
    let (rect, response) = ui.allocate_exact_size(area_size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let map_rect = egui::Rect::from_min_max(
        Pos2::new(rect.left() + 36.0, rect.top() + 22.0),
        Pos2::new(rect.right() - 12.0, rect.bottom() - 22.0),
    );
    draw_map_grid(&painter, map, rect, map_rect);
    let position = |id| cell_position(map, id, map_rect);
    let radius = |id| cell_radius(map, id, map_rect);
    let handle_position = |id: u64, direction: Direction| -> Option<Pos2> {
        let center = position(id)?;
        let cell_radius = radius(id)?;
        let offset = match &map.layout {
            MapLayout::Grid { .. } => {
                let (cell_width, cell_height) = grid_cell_size(map, map_rect)?;
                match direction {
                    Direction::West => Vec2::new(-cell_width / 2.0, 0.0),
                    Direction::North => Vec2::new(0.0, -cell_height / 2.0),
                    Direction::East => Vec2::new(cell_width / 2.0, 0.0),
                    Direction::South => Vec2::new(0.0, cell_height / 2.0),
                }
            }
            MapLayout::Graph => direction_vector(direction) * (cell_radius + 5.0),
        };
        Some(center + offset)
    };

    for (source, cell) in &map.cells {
        let exits = &cell.exits;
        for (direction, destination) in exits.iter() {
            let Some(source_handle) = handle_position(*source, direction) else {
                continue;
            };
            let Some(destination_position) = position(destination) else {
                continue;
            };
            draw_edge_from_handle(
                &painter,
                source_handle,
                destination_position,
                *source == selected,
                radius(destination).unwrap_or(17.0) + 2.0,
            );
        }
    }

    for (id, cell) in &map.cells {
        let Some(node_position) = position(*id) else {
            continue;
        };
        let color = if *id == selected {
            Color32::from_rgb(235, 145, 35)
        } else if cell.exits.iter().next().is_some() {
            Color32::from_rgb(65, 125, 205)
        } else {
            Color32::from_rgb(105, 115, 130)
        };
        painter.circle_filled(node_position, radius(*id).unwrap_or(17.0), color);
        painter.text(
            node_position,
            Align2::CENTER_CENTER,
            id.to_string(),
            FontId::proportional(12.0),
            Color32::WHITE,
        );
    }

    let handles = [
        Direction::West,
        Direction::North,
        Direction::East,
        Direction::South,
    ];
    for direction in handles {
        if let Some(handle_position) = handle_position(selected, direction) {
            let active = active_drag == Some((selected, direction));
            let handle_radius = if matches!(&map.layout, MapLayout::Grid { .. }) {
                (radius(selected).unwrap_or(17.0) * 0.55).max(4.0)
            } else {
                9.0
            };
            let fill = if active {
                Color32::from_rgb(255, 190, 60)
            } else {
                Color32::from_rgb(245, 245, 250)
            };
            painter.circle_filled(handle_position, handle_radius, fill);
            painter.circle_stroke(
                handle_position,
                handle_radius,
                Stroke::new(2.0_f32, Color32::from_rgb(35, 45, 65)),
            );
        }
    }

    if response.dragged()
        && let (Some((source, direction)), Some(pointer)) =
            (active_drag, response.interact_pointer_pos())
        && let Some(handle) = handle_position(source, direction)
    {
        draw_edge_from_handle(&painter, handle, pointer, true, 0.0);
    }

    if response.drag_started()
        && let Some(pointer) = ui.ctx().input(|input| input.pointer.press_origin())
        && let Some(direction) = handles.into_iter().find(|direction| {
            handle_position(selected, *direction).is_some_and(|handle| {
                handle.distance(pointer) <= handle_hit_radius(map, selected, map_rect)
            })
        })
    {
        return Some(EditorGridInteraction::HandleDragStarted(
            selected, direction,
        ));
    }
    let primary_released = ui
        .ctx()
        .input(|input| input.pointer.button_released(egui::PointerButton::Primary));
    if active_drag.is_some() && (response.drag_stopped() || primary_released) {
        let pointer = response.interact_pointer_pos().or_else(|| {
            ui.ctx()
                .input(|input| input.pointer.interact_pos().or(input.pointer.latest_pos()))
        });
        let target = pointer.and_then(|pointer| cell_at_position(map, map_rect, pointer));
        return Some(EditorGridInteraction::HandleDragEnded(target));
    }
    if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        if let Some(direction) = handles.into_iter().find(|direction| {
            handle_position(selected, *direction).is_some_and(|handle| {
                handle.distance(pointer) <= handle_hit_radius(map, selected, map_rect)
            })
        }) {
            return Some(EditorGridInteraction::HandleClicked(selected, direction));
        }
        if let Some(id) = cell_at_position(map, map_rect, pointer) {
            return Some(EditorGridInteraction::NodeClicked(id));
        }
    }
    None
}

fn draw_topology(
    ui: &mut egui::Ui,
    map: &PortalMap,
    route: Option<&Route>,
    start_text: &str,
    destination_text: &str,
    drag_start: Option<u64>,
) -> Option<GridInteraction> {
    let area_width = ui.available_width().max(320.0).min(760.0);
    let area_size = Vec2::new(area_width, 460.0);
    let (rect, response) = ui.allocate_exact_size(area_size, Sense::click_and_drag());
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let painter = ui.painter_at(rect);
    let left_padding = 36.0;
    let top_padding = 22.0;
    let right_padding = 12.0;
    let bottom_padding = 22.0;
    let map_rect = egui::Rect::from_min_max(
        Pos2::new(rect.left() + left_padding, rect.top() + top_padding),
        Pos2::new(rect.right() - right_padding, rect.bottom() - bottom_padding),
    );
    draw_map_grid(&painter, map, rect, map_rect);
    let position = |id| cell_position(map, id, map_rect);
    let radius = |id| cell_radius(map, id, map_rect);
    let graph = map.graph();

    for (source, exits) in &graph {
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
            draw_edge_with_insets(
                &painter,
                source_position,
                destination_position,
                highlighted,
                radius(*source).unwrap_or(17.0),
                radius(destination).unwrap_or(17.0) + 2.0,
            );
        }
    }

    if response.dragged()
        && let (Some(start), Some(pointer)) = (drag_start, response.interact_pointer_pos())
        && let Some(start_position) = position(start)
        && map_rect.contains(pointer)
    {
        draw_edge_with_insets(
            &painter,
            start_position,
            pointer,
            true,
            radius(start).unwrap_or(17.0),
            0.0,
        );
    }

    let start = parse_cell_id(start_text, "").ok();
    let destination = parse_cell_id(destination_text, "").ok();
    for id in graph.keys() {
        let Some(position) = position(*id) else {
            continue;
        };
        let color = if Some(*id) == start {
            Color32::from_rgb(55, 160, 95)
        } else if Some(*id) == destination {
            Color32::from_rgb(205, 75, 75)
        } else if route.is_some_and(|route| route.zones.contains(id)) {
            Color32::from_rgb(65, 125, 205)
        } else {
            Color32::from_rgb(105, 115, 130)
        };
        painter.circle_filled(position, radius(*id).unwrap_or(17.0), color);
        painter.text(
            position,
            Align2::CENTER_CENTER,
            id.to_string(),
            FontId::proportional(12.0),
            Color32::WHITE,
        );
    }

    if response.drag_started()
        && let Some(pointer) = response.interact_pointer_pos()
        && let Some(zone) = cell_at_position(map, map_rect, pointer)
    {
        return Some(GridInteraction::DragStarted(zone));
    }
    if response.drag_stopped() {
        let zone = response
            .interact_pointer_pos()
            .and_then(|pointer| cell_at_position(map, map_rect, pointer));
        return Some(GridInteraction::DragEnded(zone));
    }
    if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
        && let Some(zone) = cell_at_position(map, map_rect, pointer)
    {
        return Some(GridInteraction::Click(zone));
    }
    None
}

fn draw_edge_from_handle(
    painter: &egui::Painter,
    source: Pos2,
    destination: Pos2,
    highlighted: bool,
    destination_inset: f32,
) {
    draw_edge_with_insets(
        painter,
        source,
        destination,
        highlighted,
        0.0,
        destination_inset,
    );
}

fn draw_edge_with_insets(
    painter: &egui::Painter,
    source: Pos2,
    destination: Pos2,
    highlighted: bool,
    source_inset: f32,
    destination_inset: f32,
) {
    let delta = destination - source;
    let length = delta.length();
    if length < source_inset + destination_inset + 8.0 {
        return;
    }
    let unit = delta / length;
    let color = if highlighted {
        Color32::from_rgb(235, 145, 35)
    } else {
        Color32::from_gray(145)
    };
    let stroke = Stroke::new(if highlighted { 3.0_f32 } else { 1.3_f32 }, color);
    let start = source + unit * source_inset;
    let end = destination - unit * destination_inset;
    let perpendicular = Vec2::new(-unit.y, unit.x);
    let head_length = 8.0;
    let head_half_width = 4.0;
    let head_base = end - unit * head_length;
    painter.line_segment([start, head_base], stroke);
    painter.add(egui::Shape::convex_polygon(
        vec![
            end,
            head_base + perpendicular * head_half_width,
            head_base - perpendicular * head_half_width,
        ],
        color,
        Stroke::NONE,
    ));
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
    use super::{
        Pos2, Vec2, cell_at_position, cell_position, egui, ensure_portal_directory, json_filename,
        toggled_target,
    };
    use crate::map::{MAP_VERSION, MapLayout, PortalMap};
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn pointer_positions_select_grid_cells_and_reject_outside_points() {
        let map = PortalMap::new_grid(5, 5).unwrap();
        let grid = egui::Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(500.0, 500.0));
        assert_eq!(
            cell_at_position(&map, grid, Pos2::new(150.0, 250.0)),
            Some(23)
        );
        assert_eq!(cell_at_position(&map, grid, Pos2::new(500.0, 250.0)), None);
    }

    #[test]
    fn ungridded_cells_have_deterministic_positions_and_can_be_selected() {
        let mut map = PortalMap::new_graph();
        map.add_graph_cell().unwrap();
        map.add_graph_cell().unwrap();
        map.add_graph_cell().unwrap();
        let bounds = egui::Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(500.0, 400.0));
        let position = cell_position(&map, 2, bounds).unwrap();
        assert_eq!(cell_at_position(&map, bounds, position), Some(2));
        assert_eq!(map.layout, MapLayout::Graph);
    }

    #[test]
    fn editor_starts_with_all_twenty_five_grid_cells() {
        assert_eq!(PortalMap::new_grid(5, 5).unwrap().cells.len(), 25);
    }

    #[test]
    fn filling_a_sparse_grid_preserves_existing_cells_and_exits() {
        let mut map = PortalMap::from_json(r#"{"12":{"east":33},"33":{}}"#).unwrap();
        map.resize_grid(5, 5).unwrap();
        assert_eq!(map.cells.len(), 25);
        assert_eq!(map.cells[&12].exits.east, 33);
        assert!(map.cells.values().any(|cell| cell.exits.east == 0));
    }

    #[test]
    fn editor_map_serializes_with_versioned_layout_and_numeric_cells() {
        let mut map = PortalMap::new_grid(2, 1).unwrap();
        let id = map.cell_at(1, 1).unwrap();
        let target = map.cell_at(2, 1).unwrap();
        map.cells.get_mut(&id).unwrap().exits.north = target;
        let json = serde_json::to_value(map).unwrap();
        assert_eq!(json["version"], MAP_VERSION);
        assert_eq!(json["layout"]["type"], "grid");
        assert_eq!(json["cells"][id.to_string()]["north"], target);
        assert_eq!(json["cells"][id.to_string()]["x"], 1);
    }

    #[test]
    fn dragging_to_the_current_target_clears_that_direction() {
        assert_eq!(toggled_target(33, 33), (0, true));
        assert_eq!(toggled_target(33, 44), (44, false));
    }

    #[test]
    fn json_filename_adds_extension_and_rejects_paths() {
        assert_eq!(json_filename("my-map"), Some("my-map.json".to_owned()));
        assert_eq!(json_filename("my-map.JSON"), Some("my-map.json".to_owned()));
        assert_eq!(json_filename("../map"), None);
        assert_eq!(json_filename(""), None);
    }

    #[test]
    fn creates_adjacent_portal_folder_with_example_without_overwriting() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "portal-solver-portals-test-{}-{unique}",
            std::process::id()
        ));

        let created = ensure_portal_directory(&directory).unwrap();
        assert_eq!(created, directory);
        let example_path = directory.join("example.json");
        assert_eq!(
            fs::read_to_string(&example_path).unwrap(),
            include_str!("../portals/example.json")
        );

        fs::write(&example_path, b"custom map").unwrap();
        ensure_portal_directory(&directory).unwrap();
        assert_eq!(fs::read(&example_path).unwrap(), b"custom map");
        fs::remove_dir_all(directory).unwrap();
    }
}
