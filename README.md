# Portal Path Finder

A native desktop app for finding the fewest portal hops between numeric zones.
Portals are directed: a portal leading north from one zone does not imply that
the destination has a south portal back.

## Run

```sh
cargo run
```

The app looks for `portals/*.json` next to the executable. When run with
`cargo run`, it falls back to the repository's `portals/` directory if the
executable's adjacent directory does not exist. For a portable build, distribute
the executable with a `portals/` directory beside it. Use **Reload files** to
pick up JSON files added while the app is open.

The included `portals/example.json` can be used to try a route from zone `11`
to zone `33`. In path finder mode, click the 5×5 grid to set the start and then
the destination; clicks alternate between the two positions.

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
X followed by Y (`12` is X=1, Y=2). Existing files are not overwritten.
