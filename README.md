# Portal Solver

Version `0.1.6`

A native desktop app for finding the fewest portal hops between numeric cells
and creating/editing portal maps. The executable is named `portal-solver`.
Portals are directed: a portal leading north from one cell does not imply that
the destination has a south portal back.

## Build and run

```sh
cargo run
```

Build a standalone executable with:

```sh
cargo build --release
```

The release executable is `target/release/portal-solver`.

The app stores `portals/*.json` next to the executable in every run mode,
including `cargo run` (where the executable is under `target/debug/`). If that
folder or its `example.json` map is missing, the app creates the folder and
copies in the built-in example map. For a portable build, distribute the
executable with its `portals/` folder. Use **Reload files** to pick up JSON files
added while the app is open. Reloading clears the current path and endpoint
highlights.

The included `portals/example.json` can be used to try a route from zone `11`
to zone `33`. In path finder mode, click the 5×5 grid to set the start and then
the destination; clicks alternate between the two positions and the route is
calculated automatically when the destination is selected. Alternatively, drag
from the start tile to the destination tile to calculate the route in one
gesture; an arrow follows the pointer while dragging.

## Portal JSON format

New maps use version 3. The layout is either a rectangular `grid` with
1-based `x`/`y` coordinates, or an ungridded `graph`. In grid maps, cell IDs
encode X followed by Y (`11`, `12`, `23`, etc.); each dimension is limited to 9
to keep those IDs unambiguous. In ungridded maps, IDs are independent numeric
labels. ID `0` is reserved to mean no portal. Each exit points to another cell
ID in the same map. Missing directions default to `0`.

```json
{
  "version": 3,
  "layout": { "type": "grid", "width": 4, "height": 2 },
  "cells": {
    "11": { "x": 1, "y": 1, "east": 42 },
    "42": { "x": 4, "y": 2, "north": 11 }
  }
}
```

An ungridded map uses `"layout": { "type": "graph" }`; its `cells` object
contains only numeric IDs and exits. The app automatically arranges these cells
in a graph view. For example:

```json
{
  "version": 3,
  "layout": { "type": "graph" },
  "cells": {
    "7": { "east": 88 },
    "88": { "south": 7 }
  }
}
```

Every nonzero destination must refer to a cell in the file.

Existing unversioned maps remain supported. In that legacy format, the root
object maps numeric zone IDs to exits and each ID encodes its 5x5 position as X
followed by Y (`12` means X=1, Y=2). Legacy maps load without being rewritten;
saving one from the editor writes it in the current format with the same
coordinate-based IDs. Older versioned maps are upgraded in memory through
ordered migration stages: legacy syntax to version 2, then version 2 to version
3. The latter converts grid IDs to coordinate-based IDs and updates portal
destinations. Saving writes the current format; simply loading a map never
rewrites its file.

When changing map syntax, increment `MAP_VERSION` and add a dedicated migration
from the previous version in `src/map_migrations.rs`. Each stage should upgrade
only one version and be covered by migration tests; the loader applies stages in
order and rejects versions without a defined path.

Arrows show directed portals, which may connect non-adjacent coordinates. The
pathfinder displays only arrows belonging to the current route. The route's full
direction list and cell sequence appear below the grid. It chooses the route
with the fewest portals; when routes have equal length, it prefers the most
consecutive opposite-direction pairs (for example, north followed by south).
It never chooses a longer route just to add a doubling-back pair.

## Creating a portal map

Choose **Map editor** from the mode selector. Create a grid or ungridded map, or
load an existing map. Grid dimensions can be resized up to 9 cells per
dimension; cells outside a smaller resized grid are removed and portals pointing
to removed cells are cleared. Ungridded maps let you add or remove cells, and
the remaining cells are arranged automatically. Select a cell, then set its
West, North, East, and South target IDs (`0` or blank means no portal). Choose
**Apply targets** to preview field edits. The selected cell's outgoing arrows
are highlighted. Each edge has a circular direction handle: click a handle and
then a destination tile, or drag the handle to a tile, to set the target
visually. Dragging a handle onto its existing destination clears that portal
back to `0`. In an ungridded map, edit the selected cell's numeric ID and choose
**Rename selected cell** to change its label; all portals pointing to that cell
are updated to use the new ID. IDs must be unique and nonzero.

To edit an existing map, select it under **Existing map** and choose **Load for
editing**. Use **Save changes** to update that map. To create a new map, choose
**New 5x5 grid** or **New ungridded map**, enter a filename, then choose
**Create JSON file**; this always creates a separate file and does not overwrite
existing maps.

In both modes, the topology view stays above the mode's instructions, controls,
and route details so updates to those displays do not move it.

## Updating the app

At startup, the app checks GitHub for a newer stable release in the background.
If one is available, it displays **Install <version> and restart**. Selecting
that option opens a confirmation dialog; the download and installation begin
only after confirmation. When there is no update, no update button or status
message is shown. Check and installation errors are displayed in the window.

Automatic download and replacement are currently supported on Linux x86_64.
The installer validates the downloaded executable, preserves the adjacent
`portals/` directory, replaces only the app binary, and restarts the app.

Tagged releases named `v<version>` are built and published by the GitHub Actions
workflow. The tag must match the Cargo package version; the workflow checks this
and attaches the `portal-solver-linux-x86_64` executable expected by the updater.
