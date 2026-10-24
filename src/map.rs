use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::pathfinding::{PortalExits, validate_graph};

pub const MAP_VERSION: u32 = 3;
pub const MAX_GRID_DIMENSION: usize = 9;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MapLayout {
    Grid { width: usize, height: usize },
    Graph,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct MapCell {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<usize>,
    #[serde(flatten)]
    pub exits: PortalExits,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PortalMap {
    pub version: u32,
    pub layout: MapLayout,
    pub cells: BTreeMap<u64, MapCell>,
}

impl PortalMap {
    pub fn new_grid(width: usize, height: usize) -> Result<Self, String> {
        validate_dimensions(width, height)?;
        let mut map = Self {
            version: MAP_VERSION,
            layout: MapLayout::Grid { width, height },
            cells: BTreeMap::new(),
        };
        for y in 1..=height {
            for x in 1..=width {
                let id = grid_cell_id(x, y);
                map.cells.insert(
                    id,
                    MapCell {
                        x: Some(x),
                        y: Some(y),
                        exits: PortalExits::default(),
                    },
                );
            }
        }
        Ok(map)
    }

    pub fn new_graph() -> Self {
        Self {
            version: MAP_VERSION,
            layout: MapLayout::Graph,
            cells: BTreeMap::new(),
        }
    }

    pub fn from_json(contents: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(contents).map_err(|error| format!("Invalid JSON: {error}"))?;
        crate::map_migrations::upgrade(value)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != MAP_VERSION {
            return Err(format!(
                "Unsupported portal map version {}; this app supports version {}.",
                self.version, MAP_VERSION
            ));
        }
        if self.cells.is_empty() {
            return Err("A portal map must contain at least one cell.".to_owned());
        }
        if let MapLayout::Grid { width, height } = &self.layout {
            validate_dimensions(*width, *height)?;
            let mut coordinates = BTreeSet::new();
            for (id, cell) in &self.cells {
                let (Some(x), Some(y)) = (cell.x, cell.y) else {
                    return Err(format!("Grid cell {id} must specify x and y coordinates."));
                };
                if x == 0 || x > *width || y == 0 || y > *height {
                    return Err(format!(
                        "Cell {id} at ({x}, {y}) is outside the {width}×{height} grid."
                    ));
                }
                let expected_id = grid_cell_id(x, y);
                if *id != expected_id {
                    return Err(format!(
                        "Grid cell at ({x}, {y}) must use ID {expected_id}, not {id}."
                    ));
                }
                if !coordinates.insert((x, y)) {
                    return Err(format!("More than one cell occupies ({x}, {y})."));
                }
            }
        }
        if self.cells.contains_key(&0) {
            return Err("Cell ID 0 is reserved to mean no portal.".to_owned());
        }
        let graph = self.graph();
        validate_graph(&graph)
    }

    pub fn graph(&self) -> BTreeMap<u64, PortalExits> {
        self.cells
            .iter()
            .map(|(&id, cell)| (id, cell.exits.clone()))
            .collect()
    }

    pub fn next_cell_id(&self) -> u64 {
        (1..=u64::MAX)
            .find(|id| !self.cells.contains_key(id))
            .unwrap_or(1)
    }

    pub fn add_graph_cell(&mut self) -> Result<u64, String> {
        if !matches!(&self.layout, MapLayout::Graph) {
            return Err("Cells can only be added directly to ungridded maps.".to_owned());
        }
        let id = self.next_cell_id();
        self.cells.insert(id, MapCell::default());
        Ok(id)
    }

    pub fn rename_graph_cell(&mut self, current_id: u64, new_id: u64) -> Result<(), String> {
        if !matches!(&self.layout, MapLayout::Graph) {
            return Err("Cell IDs can only be changed in ungridded maps.".to_owned());
        }
        if new_id == 0 {
            return Err("Cell ID 0 is reserved to mean no portal.".to_owned());
        }
        if !self.cells.contains_key(&current_id) {
            return Err(format!("Cell {current_id} does not exist."));
        }
        if current_id != new_id && self.cells.contains_key(&new_id) {
            return Err(format!("Cell ID {new_id} is already in use."));
        }
        if current_id == new_id {
            return Ok(());
        }

        let cell = self
            .cells
            .remove(&current_id)
            .ok_or_else(|| format!("Cell {current_id} does not exist."))?;
        self.cells.insert(new_id, cell);
        for cell in self.cells.values_mut() {
            rename_target(&mut cell.exits, current_id, new_id);
        }
        Ok(())
    }

    pub fn resize_grid(&mut self, width: usize, height: usize) -> Result<usize, String> {
        validate_dimensions(width, height)?;
        let MapLayout::Grid {
            width: _,
            height: _,
        } = &self.layout
        else {
            return Err("Only grid maps can be resized.".to_owned());
        };
        let old_cells = std::mem::take(&mut self.cells);
        let old_count = old_cells.len();
        let mut retained = BTreeMap::new();
        let mut occupied = BTreeSet::new();
        for (_id, cell) in old_cells {
            let coordinates = cell.x.zip(cell.y);
            if let Some((x, y)) = coordinates.filter(|(x, y)| *x <= width && *y <= height) {
                occupied.insert((x, y));
                retained.insert(
                    grid_cell_id(x, y),
                    MapCell {
                        x: Some(x),
                        y: Some(y),
                        exits: cell.exits,
                    },
                );
            }
        }
        let removed = old_count.saturating_sub(retained.len());
        self.cells = retained;
        self.layout = MapLayout::Grid { width, height };
        let retained_ids: BTreeSet<u64> = self.cells.keys().copied().collect();
        for cell in self.cells.values_mut() {
            clear_missing_targets(&mut cell.exits, &retained_ids);
        }

        for y in 1..=height {
            for x in 1..=width {
                if occupied.insert((x, y)) {
                    let id = grid_cell_id(x, y);
                    self.cells.insert(
                        id,
                        MapCell {
                            x: Some(x),
                            y: Some(y),
                            exits: PortalExits::default(),
                        },
                    );
                }
            }
        }
        Ok(removed)
    }

    pub fn remove_cell(&mut self, id: u64) -> Result<(), String> {
        if !self.cells.contains_key(&id) {
            return Err(format!("Cell {id} does not exist."));
        }
        self.cells.remove(&id);
        let valid_ids: BTreeSet<u64> = self.cells.keys().copied().collect();
        for cell in self.cells.values_mut() {
            clear_missing_targets(&mut cell.exits, &valid_ids);
        }
        Ok(())
    }

    pub fn grid_dimensions(&self) -> Option<(usize, usize)> {
        match &self.layout {
            MapLayout::Grid { width, height } => Some((*width, *height)),
            MapLayout::Graph => None,
        }
    }

    pub fn cell_at(&self, x: usize, y: usize) -> Option<u64> {
        self.cells
            .iter()
            .find_map(|(&id, cell)| (cell.x == Some(x) && cell.y == Some(y)).then_some(id))
    }
}

pub(crate) fn legacy_coordinates(id: u64) -> Option<(usize, usize)> {
    let x = id / 10;
    let y = id % 10;
    if (1..=5).contains(&x) && (1..=5).contains(&y) {
        Some((x as usize, y as usize))
    } else {
        None
    }
}

pub(crate) fn grid_cell_id(x: usize, y: usize) -> u64 {
    (x * 10 + y) as u64
}

fn validate_dimensions(width: usize, height: usize) -> Result<(), String> {
    if width == 0 || height == 0 || width > MAX_GRID_DIMENSION || height > MAX_GRID_DIMENSION {
        return Err(format!(
            "Grid width and height must be between 1 and {MAX_GRID_DIMENSION} to preserve coordinate-based cell IDs."
        ));
    }
    Ok(())
}

fn clear_missing_targets(exits: &mut PortalExits, valid_ids: &BTreeSet<u64>) {
    if exits.west != 0 && !valid_ids.contains(&exits.west) {
        exits.west = 0;
    }
    if exits.north != 0 && !valid_ids.contains(&exits.north) {
        exits.north = 0;
    }
    if exits.east != 0 && !valid_ids.contains(&exits.east) {
        exits.east = 0;
    }
    if exits.south != 0 && !valid_ids.contains(&exits.south) {
        exits.south = 0;
    }
}

fn rename_target(exits: &mut PortalExits, old_id: u64, new_id: u64) {
    for target in [
        &mut exits.west,
        &mut exits.north,
        &mut exits.east,
        &mut exits.south,
    ] {
        if *target == old_id {
            *target = new_id;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MapLayout, PortalMap};

    #[test]
    fn converts_legacy_maps_in_memory_and_serializes_versioned_grid() {
        let legacy = r#"{"12":{"east":21},"21":{"west":0}}"#;
        let map = PortalMap::from_json(legacy).unwrap();
        assert_eq!(
            map.layout,
            MapLayout::Grid {
                width: 5,
                height: 5
            }
        );
        assert_eq!(map.cells[&12].x, Some(1));
        assert_eq!(map.cells[&12].y, Some(2));
        assert_eq!(map.cells[&12].exits.east, 21);
        let serialized = serde_json::to_value(map).unwrap();
        assert_eq!(serialized["version"], super::MAP_VERSION);
        assert_eq!(serialized["cells"]["12"]["x"], 1);
    }

    #[test]
    fn bundled_legacy_example_remains_usable() {
        let map = PortalMap::from_json(include_str!("../portals/example.json")).unwrap();
        assert_eq!(map.cells.len(), 5);
        assert_eq!(map.cells[&12].x, Some(1));
        assert_eq!(map.cells[&12].y, Some(2));
        assert_eq!(map.graph()[&11].west, 12);
    }

    #[test]
    fn new_rectangular_grids_use_x_then_y_cell_ids() {
        let map = PortalMap::new_grid(3, 3).unwrap();
        assert_eq!(map.cell_at(1, 1), Some(11));
        assert_eq!(map.cell_at(1, 2), Some(12));
        assert_eq!(map.cell_at(2, 3), Some(23));
        assert_eq!(map.cell_at(3, 2), Some(32));
    }

    #[test]
    fn normalizes_versioned_grid_ids_to_coordinate_ids_and_updates_edges() {
        let json = r#"{
          "version": 2,
          "layout": {"type": "grid", "width": 4, "height": 2},
          "cells": {
            "900": {"x": 1, "y": 2, "east": 300},
            "300": {"x": 4, "y": 1, "west": 900}
          }
        }"#;
        let map = PortalMap::from_json(json).unwrap();
        assert_eq!(map.grid_dimensions(), Some((4, 2)));
        assert_eq!(map.cell_at(1, 2), Some(12));
        assert_eq!(map.cell_at(4, 1), Some(41));
        assert_eq!(map.cells[&12].exits.east, 41);
        assert_eq!(map.cells[&41].exits.west, 12);
        let encoded = serde_json::to_string(&map).unwrap();
        assert_eq!(PortalMap::from_json(&encoded).unwrap(), map);
    }

    #[test]
    fn accepts_ungridded_maps_with_sparse_numeric_ids() {
        let json = r#"{
          "version": 2,
          "layout": {"type": "graph"},
          "cells": {
            "7": {"east": 88},
            "88": {}
          }
        }"#;
        let map = PortalMap::from_json(json).unwrap();
        assert_eq!(map.grid_dimensions(), None);
        assert_eq!(map.cells.len(), 2);
        assert_eq!(map.graph()[&7].east, 88);
    }

    #[test]
    fn rejects_unknown_versions_bad_coordinates_and_missing_targets() {
        assert!(
            PortalMap::from_json(r#"{"version":99,"layout":{"type":"graph"},"cells":{}}"#).is_err()
        );
        assert!(PortalMap::from_json(
            r#"{"version":2,"layout":{"type":"grid","width":2,"height":2},"cells":{"1":{"x":3,"y":1}}}"#
        )
        .is_err());
        assert!(
            PortalMap::from_json(
                r#"{"version":2,"layout":{"type":"graph"},"cells":{"1":{"west":2}}}"#
            )
            .is_err()
        );
        assert!(
            PortalMap::from_json(
                r#"{"version":2,"layout":{"type":"grid","width":10,"height":1},"cells":{"11":{"x":1,"y":1}}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn graph_cell_removal_clears_incoming_edges() {
        let mut map = PortalMap::from_json(
            r#"{"version":2,"layout":{"type":"graph"},"cells":{"1":{"east":2},"2":{}}}"#,
        )
        .unwrap();
        map.remove_cell(2).unwrap();
        assert_eq!(map.cells[&1].exits.east, 0);
    }

    #[test]
    fn renaming_graph_cell_updates_all_portal_references() {
        let mut map = PortalMap::from_json(
            r#"{"version":2,"layout":{"type":"graph"},"cells":{"1":{"east":2},"2":{"west":1,"north":2}}}"#,
        )
        .unwrap();
        map.rename_graph_cell(2, 99).unwrap();
        assert!(!map.cells.contains_key(&2));
        assert_eq!(map.cells[&1].exits.east, 99);
        assert_eq!(map.cells[&99].exits.west, 1);
        assert_eq!(map.cells[&99].exits.north, 99);
        assert!(map.validate().is_ok());
    }

    #[test]
    fn renaming_rejects_zero_duplicate_and_missing_ids() {
        let mut map = PortalMap::from_json(
            r#"{"version":2,"layout":{"type":"graph"},"cells":{"1":{},"2":{}}}"#,
        )
        .unwrap();
        assert!(map.rename_graph_cell(1, 0).is_err());
        assert!(map.rename_graph_cell(1, 2).is_err());
        assert!(map.rename_graph_cell(3, 4).is_err());
        assert_eq!(map.cells.len(), 2);
    }

    #[test]
    fn resizing_clears_portals_before_reusing_removed_cell_ids() {
        let mut map = PortalMap::new_grid(2, 1).unwrap();
        map.cells.get_mut(&11).unwrap().exits.east = 21;
        assert_eq!(map.resize_grid(1, 1).unwrap(), 1);
        assert_eq!(map.cells[&11].exits.east, 0);
        map.resize_grid(2, 1).unwrap();
        assert_eq!(map.cells[&11].exits.east, 0);
        assert_eq!(map.cell_at(2, 1), Some(21));
    }
}
