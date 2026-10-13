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
- **Open / import project** selects an existing folder containing `scene.json`.
  This opens projects in place without changing or
  copying their existing data.
- The Home page lists up to 12 recently opened projects. Open one directly, remove
  an entry, or clear the list. Recent projects are stored in your user application
  data (`%APPDATA%\Altadra\recent-projects.json` on Windows).
- **Home** in the editor returns to the project picker; unsaved changes require
  confirmation. Exiting an unsaved project or a project with recent edits asks
  whether to save, discard, or cancel. Existing projects save to their current
  folder; new projects open the Save project dialog.
- New projects contain no sample sprites or assets. Add sprites from the Scene
  tab, and import PNG assets from files anywhere on your computer.
- The editor's left dock opens to the **Project** tab, which shows the active
  project's files and folders directly; the project folder itself is not a tree
  row, and its name is shown as the panel header next to the **+** menu. Unsaved
  items are shown in yellow with a trailing `*` (every item in a never-saved
  project, newly created files and folders until the next save, the open text
  file with unsaved edits and its parent folders, and `scene.json` when the
  scene has unsaved changes); they return to normal once saved. As in VS Code,
  new files and folders are created on disk immediately; the marker shows they
  have not been saved yet. Create files and folders, rename or delete selected
  entries, and click UTF-8 text files to edit them. Text edits are limited to
  1 MB and save from the file editor or project Save button. Unsaved projects
  keep files in a temporary workspace and copy them into the chosen project
  folder when saved. `scene.json` remains managed by the scene editor; binary
  assets can be imported from anywhere. Use the **+** menu or right-click the
  project root, a folder, or a file to create items; a name field appears inline
  in the tree where the item will be added (press **Enter** to create, **Esc** to
  cancel). Right-click a folder or file to rename it in place the same way, or to
  delete it; files only offer **New File** (no New Folder). Select a file or
  folder and press **Delete** to
  remove it (with confirmation). Selected sprites have a bright outline in the
  viewport. Switch to **Scene** for the sprite hierarchy and controls.
- Edit name, position, rotation, scale, size, tint, visibility, velocity, and each
  sprite's Lua script in the inspector.
- Click sprites to select them in the viewport. Rotation and scale are used for
  rendering and rectangle picking. The scene origin is centered; +X is right,
  +Y is down; the last sprite draws on top.
- Import PNG files. Images are copied into the project on save. They can be tinted
  in the inspector. Without an imported image, sprites render as solid-color quads.
- New, Open, Save, and Save As operate on project directories. Save As uses the
  same Save project dialog: the chosen folder is the location, and a new project
  folder with the given name is created inside it (existing folders are never
  overwritten). Unsaved changes require confirmation. Press **Ctrl+S** to save
  the project (and any open text file); unsaved new projects open the Save
  project dialog.
- Play runs a fixed 60 Hz clone of the authoring scene. Click the viewport and use
  WASD or arrow keys; Stop discards runtime changes and preserves the authored scene.
- Script/runtime, file, and image errors are shown in the editor status area.

## Lua scripts

Each sprite has a Lua script which returns a function:

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
`dt` is the fixed 1/60-second step. Scripts are stored in the scene JSON and run
only in Play mode. Base `os`, `io`, and `debug` Lua libraries are not enabled.
Each invocation is limited to 100,000 Lua instructions and the runtime has a
64 MiB Lua memory budget; non-finite outputs and runtime errors stop play with
an explicit error. Lua scripts do not yet have
asset, audio, scene-transition, or entity-creation APIs.

## Project format

```text
MyGame\
  scene.json
  assets\          (created only when PNG images are imported)
    <sha256>.png
```

Versioned JSON stores sprite transforms, tint, velocity, and Lua source. Imported
PNG files are content-addressed under `assets/` and use relative references, so
projects can be copied or moved. Scene JSON is limited to 16 MB, sprites to
10,000 per scene, PNG files to 64 MB, and image dimensions to 4096 x 4096.
Path traversal and symlink paths are rejected. Save writes the updated scene
through a temporary file after copying referenced images.

## Tests and CI

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

Windows CI runs formatting, tests, Clippy, and release builds.

## Scope

This is an initial engine/editor foundation, not a full production engine. The
runtime includes fixed-step simulation and sandboxed per-sprite Lua scripts.
Physics, animation, audio, undo/redo, autosave, packaging installers, and
standalone game export are not yet included.
