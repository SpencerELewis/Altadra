# Altadra

A starter 2D game engine and Windows scene editor written in **Rust**, with
**Lua 5.4** for gameplay scripts. The desktop editor uses `eframe`/`egui`.

## Prerequisites

- Rust stable and Cargo (`rustup` installs both):

  ```powershell
  winget install Rustlang.Rustup
  rustup default stable-x86_64-pc-windows-msvc
  ```

- Visual Studio Build Tools with **Desktop development with C++** and a Windows SDK.
  The `mlua` dependency builds Lua 5.4 from source, so the MSVC C/C++ build tools
  are required for Windows builds.

## Build and launch

```powershell
cargo run
```

Build a release Windows app:

```powershell
cargo build --release
```

The release executable is `target\release\altadra.exe`. It uses the Windows GUI
subsystem, so it does not open an extra console window.

## Editor

- The app opens to **Home**. Name a project and select **Create project** to open
  an unsaved, empty project; choosing a save location up front is optional. The
  first save (from the toolbar or when exiting) opens a **Save project** dialog:
  enter the project name and choose a location, and Altadra creates the project
  folder `<location>\<name>` inside it.
- **Open / import project** selects an existing folder containing one `.altgame`
  project settings file. Legacy folders containing `scene.json` prompt for
  conversion; original files are retained and conflicting outputs are rejected.
  This opens projects in place without changing or
  copying their existing data.
- The Home page lists up to 12 recently opened projects. Open one directly, remove
  an entry, or clear the list. Recent projects are stored in your user application
  data (`%APPDATA%\Altadra\recent-projects.json` on Windows).
- Click **ALTADRA** in the editor's top bar to return to the project picker; unsaved changes require
  confirmation. Exiting an unsaved project or a project with recent edits asks
  whether to save, discard, or cancel. Existing projects save to their current
  folder; new projects open the Save project dialog.
- New projects contain project settings, but no sample objects, levels, or assets.
  Use **+ > New Object**, **New Level**, or **New Behavior** to create custom
  files with valid initial contents. Names are entered inline; their extensions
  are added automatically. Ordinary New File and New Folder remain available.
- The editor's left dock opens to the **Project** tab, which shows the active
  project's files and folders directly; the project folder itself is not a tree
  row, and its name is shown as the panel header next to the **+** menu. Unsaved
  items are shown in yellow with a trailing `*` (every item in a never-saved
  project, newly created files and folders until the next save, open text
  files with unsaved edits and their parent folders); they return to normal once
  saved. As in VS Code,
  new files and folders are created on disk immediately; the marker shows they
  have not been saved yet. Create files and folders, rename or delete selected
  entries, and click UTF-8 text files to edit them. Text edits are limited to
  1 MB and save from the file editor or project Save button. Unsaved projects
  keep files in a temporary workspace and copy them into the chosen project
  folder when saved. The project settings filename is managed by Altadra; binary
  assets can be imported from anywhere. Use the **+** menu or right-click the
  project root, a folder, or a file to create items; a name field appears inline
  in the tree where the item will be added (press **Enter** to create, **Esc** to
  cancel). Right-click a folder or file to rename it in place the same way, or to
  delete it; files only offer **New File** (no New Folder). Select a file or
  folder and press **Delete** to
  remove it (with confirmation). Selected sprites have a bright outline in the
  viewport. Levels are ordinary document tabs, not a separate Scene dock.
- Open text files appear as tabs across the top of the file editor. Click a tab
  to switch without losing unsaved edits; opening the same file again selects
  its existing tab. Each tab has an **x** to close it; closing an edited file
  asks whether to save, discard, or cancel. Unsaved tabs have yellow names and
  a trailing `*`, and hovering a tab shows its full path. The tab strip scrolls
  horizontally when needed. Closing the last tab returns to the scene viewport.
  **Save file** saves only the active tab; project **Save** saves all open files.
- Single-clicking a file opens an italic preview tab; opening another preview
  replaces it. Double-click a file or its tab to keep it open. Editing a preview
  also keeps it open automatically; unsaved edits are never replaced.
- Object tabs edit reusable appearance and velocity defaults, PNG references, and
  external Lua behavior references in Properties. Level tabs show a placement
  canvas and an instance list in Properties. Add objects using the object picker
  or drag `.altobject` files from the explorer onto the canvas. Select and drag
  instances to move them; Properties also supports transforms, duplication,
  deletion, optional overrides, and resetting overrides to object defaults.
- Project tabs select the starting level. PNG tabs preview artwork. Invalid custom
  documents show an error; **Edit TOML source** allows inspection and repair.
- Renaming project files updates referencing documents and marks them unsaved;
  deleting referenced items warns which documents will have broken references.
- Click sprites to select them in the viewport. Rotation and scale are used for
  rendering and rectangle picking. The scene origin is centered; +X is right,
  +Y is down; the last sprite draws on top.
- Import PNG files to copy artwork into the project. If an object tab is active,
  the imported image is assigned to it. Objects without an image render as
  solid-color rectangles.
- **File** in the top bar contains **New**, **Open**, **Save**, and **Save As**,
  which operate on project directories. Save As uses the
  same Save project dialog: the chosen folder is the location, and a new project
  folder with the given name is created inside it (existing folders are never
  overwritten). Unsaved changes require confirmation. Press **Ctrl+S** to save
  the project (and all open text files); unsaved new projects open the Save
  project dialog.
- Play runs the active level, or the project's configured starting level when
  another file is active. Unsaved object, level, and Lua buffers are included in
  the runtime snapshot. Play runs a fixed 60 Hz clone; click the viewport and use
  WASD or arrow keys; Stop discards runtime changes and preserves the authored scene.
- Script/runtime, file, and image errors are shown in the editor status area.

## Lua scripts

An object can reference a `.lua` behavior file which returns a function:

```lua
return function(entity, input, dt)
    local speed = 180
    if input.left then entity.x = entity.x - speed * dt end
    if input.right then entity.x = entity.x + speed * dt end
    if input.up then entity.y = entity.y - speed * dt end
    if input.down then entity.y = entity.y + speed * dt end
end
```

`entity` exposes mutable `x`, `y`, `vx`, and `vy`, plus read-only-by-convention
`width` and `height`. `input` exposes `left`, `right`, `up`, `down`, `x`, and `y`.
`dt` is the fixed 1/60-second step. Behavior files run only in Play mode.
Base `os`, `io`, and `debug` Lua libraries are not enabled.
Each invocation is limited to 100,000 Lua instructions and the runtime has a
64 MiB Lua memory budget; non-finite outputs and runtime errors stop play with
an explicit error. Lua scripts do not yet have
asset, audio, scene-transition, or entity-creation APIs.

## Project format

Custom files use versioned TOML. The root has one `.altgame` manifest; other files
can be organized in ordinary folders. References use project-relative `/`
separators on disk so projects remain portable.

```text
MyGame\
  game.altgame
  Player.altobject
  Player.lua
  Level1.altscene
  assets\          (created only when PNG images are imported)
    <sha256>.png
```

`.altgame` stores the project name and optional `starting_level`. `.altobject`
stores `appearance`, `behavior`, and `motion` sections. `.altscene` stores
`[[instances]]` with unique IDs, object references, position/rotation/scale, and
optional `overrides`. Multiple instances share object defaults; an explicit
override affects only that instance.

```toml
# Player.altobject
version = 1
name = "Player"

[appearance]
width = 32
height = 48
tint = [255, 255, 255, 255]
visible = true

[behavior]
script = "Player.lua"

[motion]
vx = 0
vy = 0
```

TOML/text files are limited to 1 MB, Lua behaviors to 64 KB, instances to 10,000
per level, PNG files to 64 MB, and image dimensions to 4096 x 4096. Unknown fields
and unsupported versions are rejected. Path traversal and symlinks are rejected;
text saves are atomic. Legacy migration keeps the old JSON and asset files.

## Tests and CI

```powershell
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked
```

Windows CI runs formatting, tests, Clippy, and release builds. Architecture notes,
invariants, and contributor/AI-agent guidance are in [AGENTS.md](AGENTS.md).

## Scope

This is an initial engine/editor foundation, not a full production engine. The
runtime includes fixed-step simulation and sandboxed per-sprite Lua scripts.
Physics, animation, audio, undo/redo, autosave, packaging installers, and
standalone game export are not yet included.
