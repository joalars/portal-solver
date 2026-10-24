use std::collections::BTreeMap;

use serde_json::Value;

use crate::{
    map::{MAP_VERSION, MapCell, MapLayout, PortalMap, grid_cell_id, legacy_coordinates},
    pathfinding::{PortalExits, validate_graph},
};

const FIRST_VERSIONED_MAP_VERSION: u32 = 2;

pub fn upgrade(value: Value) -> Result<PortalMap, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "Portal map must be a JSON object.".to_owned())?;
    let mut map = if object.contains_key("version") {
        serde_json::from_value(value)
            .map_err(|error| format!("Invalid versioned portal map: {error}"))?
    } else {
        migrate_legacy_to_v2(value)?
    };

    while map.version < MAP_VERSION {
        map = match map.version {
            2 => migrate_v2_to_v3(map)?,
            version => {
                return Err(format!(
                    "No portal-map migration is defined from version {version} to version {}.",
                    version + 1
                ));
            }
        };
    }
    if map.version > MAP_VERSION {
        return Err(format!(
            "Unsupported portal map version {}; this app supports version {}.",
            map.version, MAP_VERSION
        ));
    }

    map.validate()?;
    Ok(map)
}

fn migrate_legacy_to_v2(value: Value) -> Result<PortalMap, String> {
    let graph: BTreeMap<u64, PortalExits> = serde_json::from_value(value)
        .map_err(|error| format!("Invalid legacy portal map: {error}"))?;
    validate_graph(&graph)?;

    let mut cells = BTreeMap::new();
    for (id, exits) in graph {
        let (x, y) = legacy_coordinates(id).ok_or_else(|| {
            format!("Legacy zone ID {id} is not a coordinate within the 5×5 grid.")
        })?;
        cells.insert(
            id,
            MapCell {
                x: Some(x),
                y: Some(y),
                exits,
            },
        );
    }

    Ok(PortalMap {
        version: FIRST_VERSIONED_MAP_VERSION,
        layout: MapLayout::Grid {
            width: 5,
            height: 5,
        },
        cells,
    })
}

fn migrate_v2_to_v3(mut map: PortalMap) -> Result<PortalMap, String> {
    if let MapLayout::Grid { width, height } = &map.layout {
        if *width == 0
            || *height == 0
            || *width > crate::map::MAX_GRID_DIMENSION
            || *height > crate::map::MAX_GRID_DIMENSION
        {
            return Err(format!(
                "Version 2 grid dimensions must be between 1 and {}.",
                crate::map::MAX_GRID_DIMENSION
            ));
        }

        let mut id_remap = BTreeMap::new();
        for (&old_id, cell) in &map.cells {
            if old_id == 0 {
                return Err("Cell ID 0 is reserved to mean no portal.".to_owned());
            }
            let (Some(x), Some(y)) = (cell.x, cell.y) else {
                return Err(format!(
                    "Grid cell {old_id} must specify x and y coordinates."
                ));
            };
            if x == 0 || y == 0 || x > *width || y > *height {
                return Err(format!(
                    "Cell {old_id} at ({x}, {y}) is outside the {width}×{height} grid."
                ));
            }
            id_remap.insert(old_id, grid_cell_id(x, y));
        }

        let mut migrated_cells = BTreeMap::new();
        for (old_id, mut cell) in std::mem::take(&mut map.cells) {
            remap_targets(&mut cell.exits, &id_remap);
            let new_id = id_remap[&old_id];
            if migrated_cells.insert(new_id, cell).is_some() {
                return Err(format!(
                    "More than one grid cell occupies the coordinates for ID {new_id}."
                ));
            }
        }
        map.cells = migrated_cells;
    }

    map.version = 3;
    Ok(map)
}

fn remap_targets(exits: &mut PortalExits, id_remap: &BTreeMap<u64, u64>) {
    for target in [
        &mut exits.west,
        &mut exits.north,
        &mut exits.east,
        &mut exits.south,
    ] {
        if let Some(&new_id) = id_remap.get(target) {
            *target = new_id;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::upgrade;
    use crate::map::{MAP_VERSION, PortalMap};
    use serde_json::json;

    #[test]
    fn upgrades_unversioned_legacy_through_each_required_stage() {
        let map = PortalMap::from_json(r#"{"12":{"east":21},"21":{"west":0}}"#).unwrap();
        assert_eq!(map.version, MAP_VERSION);
        assert_eq!(map.cells[&12].exits.east, 21);
        assert_eq!(map.grid_dimensions(), Some((5, 5)));
    }

    #[test]
    fn refuses_versions_without_a_known_migration_stage() {
        let error = upgrade(json!({
            "version": 1,
            "layout": {"type": "graph"},
            "cells": {"1": {}}
        }))
        .unwrap_err();
        assert!(error.contains("No portal-map migration is defined from version 1 to version 2"));
    }

    #[test]
    fn rejects_migrations_that_collapse_two_old_cells_to_one_coordinate() {
        let error = upgrade(json!({
            "version": 2,
            "layout": {"type": "grid", "width": 2, "height": 1},
            "cells": {
                "11": {"x": 1, "y": 1},
                "90": {"x": 1, "y": 1}
            }
        }))
        .unwrap_err();
        assert!(error.contains("More than one grid cell"));
    }
}
