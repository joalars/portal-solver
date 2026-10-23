# Portal Solver

A native desktop app for finding the fewest portal hops between numeric zones
and creating/editing portal maps. The executable is named `portal-solver`.
Portals are directed: a portal leading north from one zone does not imply that
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
added while the app is open.

The included `portals/example.json` can be used to try a route from zone `11`
to zone `33`. In path finder mode, click the 5×5 grid to set the start and then
the destination; clicks alternate between the two positions and the route is
calculated automatically when the destination is selected.

## Portal JSON format

The root object maps numeric zone IDs to their exits. A zone ID encodes its grid
position as X followed by Y: `12` means X=1, Y=2. The topology view uses a
1-based 5x5 coordinate grid. Exit values are numeric zone IDs, and `0` means
there is no portal in that direction. Missing direction fields are treated as
no portal. Every nonzero destination must also exist as a zone in the same file.

```json
{
  "11": {
    "west": 12,
    "east": 21,
    "north": 0,
    "south": 0
  },
  "12": {
    "north": 22
  },
  "21": {
    "east": 22
  },
  "22": {
    "south": 33
  },
  "33": {}
}
```

Arrows show directed portals, which may connect non-adjacent coordinates. The
shortest path is highlighted in the grid and its full direction list is shown
above it. When several routes use the same minimum number of portals, the app
prefers the route with the most consecutive opposite-direction pairs (for
example, north followed by south). It never chooses a route with more portals
just to add a doubling-back pair.

## Creating a portal map

Choose **Map editor** from the mode selector. Click a node on the 5×5 grid,
enter the destination zone ID for each direction (`0` or blank means no portal),
and choose **Apply targets** to preview its links. Enter a new file name and
choose **Create JSON file** to save it in the `portals/` folder. Zone IDs use
X followed by Y (`12` is X=1, Y=2). To change an existing map, select it under
**Existing map**, choose **Load for editing**, then use **Save changes**.
**Create JSON file** always creates a separate file and will not overwrite one.

In both modes, the grid stays above the mode's instructions, controls, and route
details so updates to those displays do not move the grid.

## Updating the app

At startup, the app checks GitHub for a newer stable release in the background.
If one is available, it displays **Install <version> and restart**. Selecting
that option opens a confirmation dialog; the download and installation begin
only after confirmation. When there is no update, no update button or status
message is shown. Check failures are displayed in the window.

Automatic download and replacement are currently supported on Linux x86_64.
The installer validates the downloaded executable, preserves the adjacent
`portals/` directory, replaces only the app binary, and restarts the app.

Tagged releases named `v<version>` are built and published by the GitHub Actions
workflow. The tag must match the Cargo package version; the workflow checks this
and attaches the `portal-solver-linux-x86_64` executable expected by the updater.
