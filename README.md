# canvas

A small vector painting app. One binary runs an HTTP server that serves the drawing UI and an API, and the same binary works as a CLI client for a running server.

The document is an append-only log of actions (stroke, resize, clear, undo, redo). The current picture is the replay of that log.

## Run

```
cargo build --release
./target/release/canvas                  # serve on 127.0.0.1:7878 and open the browser
./target/release/canvas serve --port 8000 --no-open drawing.canvas
```

## HTTP API

| Method | Path | Notes |
|--------|------|-------|
| GET | `/` | UI |
| GET | `/api/state` | `doc, version, width, height, background, initial_width, initial_height, can_undo, can_redo` |
| GET | `/api/actions?since=N` | state fields plus `actions` from log index N |
| POST | `/api/actions` | one action or an array, returns `{version}` |
| POST | `/api/new` | `{width?, height?, background?}`, resets the document |
| GET | `/api/export.svg` | SVG |
| GET | `/api/export.png` | PNG, `?scale=2` |
| GET | `/api/export.jpg` | JPEG, `?scale=2&quality=90` |
| GET | `/api/project` | download `drawing.canvas` |
| POST | `/api/project` | raw `.canvas` body, replaces the document |

Actions:

```
{"type":"stroke","color":"#rrggbb","width":4,"points":[[x,y],...],"eraser":false}
{"type":"resize","left":0,"top":0,"right":100,"bottom":0}
{"type":"clear"} {"type":"undo"} {"type":"redo"}
```

The server stamps each action with `t` (unix ms). Points and widths are stored at 1/16 px.

## CLI

Client commands use `--url` or `CANVAS_URL` (default `http://127.0.0.1:7878`).

```
canvas [serve] [--port N] [--bind ADDR] [file.canvas] [--no-open]
canvas stroke [--color #hex] [--width N] [--eraser] x,y x,y ...
canvas clear | undo | redo | state
canvas resize [--left N] [--top N] [--right N] [--bottom N]
canvas new [--width N] [--height N] [--background #hex]
canvas export out.svg|out.png|out.jpg [--scale N] [--quality N]
canvas save file.canvas
canvas load file.canvas
canvas history
canvas help
```

## .canvas format

`CNVS`, a version byte (1), then a deflate stream of:

- varints: width, height; 3 bytes background RGB; varint action count
- per action: type byte (0 stroke, 1 eraser stroke, 2 resize, 3 clear, 4 undo, 5 redo), zigzag varint time delta from the previous action
- stroke: 3 color bytes, varint width in 1/16 px, varint point count, then zigzag varint deltas of the 1/16 px coordinates
- resize: zigzag varints left, top, right, bottom

Saving and loading is exact.
