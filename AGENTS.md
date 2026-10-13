# Altadra — Agent Guide

Altadra is a Windows 2D game engine + scene editor written in **Rust** (edition 2024)
with **Lua 5.4** gameplay scripts. The editor UI is `eframe`/`egui` **0.31**; Lua is
embedded via `mlua` (vendored, so it compiles Lua's C sources).

User-facing behavior, the Lua scripting API, and the on-disk project format are
documented in [README.md](README.md) — read it first and keep it in sync with any
behavior change.

## Build and test

All of these must pass before work is considered done (they mirror
[.github/workflows/windows.yml](.github/workflows/windows.yml)):

```powershell
cargo fmt
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked
```

- Building requires the MSVC C/C++ toolchain (for `mlua`). If `cl.exe`/`link.exe`
  are not found, run the commands from a Developer PowerShell, or chain them
  through `VsDevCmd.bat` in a **single** `cmd /c` invocation, e.g.
  `cmd /c 'call "<VS path>\Common7\Tools\VsDevCmd.bat" -arch=amd64 && cargo build --release --locked'`.
- A running `target\release\altadra.exe` locks the binary; stop that process (by
  PID) before rebuilding.
- The app is a GUI; there are no UI tests. Logic that can be unit-tested
  (paths, file ops, persistence, runtime) should be extracted into free functions
  or modules and covered by `#[cfg(test)]` tests using `tempfile` directories.

## Architecture (`src/`)

| Module | Responsibility |
|---|---|
| `main.rs` | The whole editor app (`Altadra` struct implementing `eframe::App`): Home page, top bar, Project/Scene left dock, inspector, viewport, text editor, save dialog, file tree rendering. |
| `project.rs` | `Project` load/save (`scene.json` + `assets/*.png`), PNG import (SHA-256 content-addressed), project/asset name validation, symlink rejection. |
| `project_files.rs` | Sandboxed file-explorer operations: create/rename/delete entries, read/write UTF-8 text (≤ 1 MB, atomic writes), copy workspace files, stage assets. Every path goes through `checked_path`. |
| `scene.rs` | `Scene`/`Sprite` data model and validation. |
| `runtime.rs` | Play mode: fixed 60 Hz step over a cloned scene, sandboxed Lua (no `os`/`io`/`debug`, instruction and memory limits). |
| `recent.rs` | Recent-projects list persisted to `%APPDATA%\Altadra\recent-projects.json`. |

### Editor state model (main.rs)

- `page: AppPage` — app opens to `Home`; `Editor` once a project is created/opened.
- New projects are **unsaved**: `project.path == None` and files live in a temporary
  workspace `file_workspace: Option<TempDir>`. `file_root_path()` returns the
  workspace if present, else `project.path`. Saving copies the workspace into the
  chosen folder and drops it.
- The save flow (`SaveProjectDialog`) treats the picked folder as the **location**
  and creates `<location>\<project name>`; it never overwrites an existing folder.
  Used by first Save, Save As, Ctrl+S on unsaved projects, and save-on-exit.
- `files: EditorFiles` holds a vector of `TextDocument`s (path, contents, dirty)
  and an active tab index. Switching tabs preserves buffers; closing dirty tabs
  prompts. Project Save writes every dirty document before copying project files;
  Save file writes only the active document. Remap all tab paths on rename,
  first save, and Save As; remove affected tabs only after successful deletion.
- Documents have a `preview` flag: single-click replaces the clean preview tab,
  double-click pins it, and editing pins it automatically. Preview labels are
  italic. Never replace a dirty buffer. The File menu holds project actions;
  clicking ALTADRA returns Home using the existing unsaved-changes prompt.
- Unsaved tracking: `dirty` (scene), each document's `dirty` flag,
  `unsaved_entries` (files/folders created since the last save). Use
  `has_unsaved_changes()` for prompts/status. The tree renders unsaved items in
  yellow with a trailing ` *`.
- `file_dialog: Option<FileDialogState>` drives **inline** create/rename fields in
  the tree (no popups); `InlineOutcome` = Enter commit / blur commit / Esc cancel.
- File-system changes happen on disk immediately (VS Code model); only contents
  and "new since last save" are tracked as unsaved.

## Conventions and gotchas

- **egui 0.31 API**: close menus with `ui.close_menu()` (`ui.close()` doesn't
  exist). `CollapsingHeader` can't render a selectable header — use
  `egui::collapsing_header::CollapsingState::load_with_default_open(..).show_header(..).body(..)`
  and attach context menus to the returned header's `.inner` response (the
  `.response` of the header row is not click-sensitive).
- **Borrowing in UI closures**: clone paths up front, record clicks into local
  flags/`Option`s inside the closure, then call `&mut self` methods after it.
  Tree rendering passes state through `TreeContext` for this reason.
- **Paths**: `project_files` canonicalizes paths, which on Windows adds a `\\?\`
  prefix. Convert results back with `tree_form_path(root, path)` before comparing
  with tree/selection paths.
- **Safety invariants**: never bypass `project_files::checked_path` for project file
  operations; reject symlinks and paths outside the project root; root
  `scene.json` is managed by the scene editor and must not be edited/renamed/deleted
  through the file explorer; block file operations during Play mode.
- The `assets` folder is created only when a PNG is imported/saved — don't
  recreate it eagerly. New projects contain no sample sprites or files.
- Errors are surfaced as `Result<_, String>` and shown in the status bar
  (`self.error`) rather than panicking.

## Working with the owner

- UI/UX decisions follow VS Code / mainstream IDE conventions unless told otherwise
  (file explorer, inline naming, context menus, Delete key, Ctrl+S).
- The owner commits changes themselves; don't commit unless asked.
- After a change, rebuild the release binary and launch it so it can be tried.
- Keep this file current when architecture, invariants, or workflow change.
