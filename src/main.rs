#![cfg_attr(
    all(windows, not(debug_assertions), not(test)),
    windows_subsystem = "windows"
)]

mod document_editor;
pub mod documents;
mod project;
mod project_files;
mod recent;
mod runtime;
mod scene;

use eframe::egui::{self, Color32, Pos2, Sense, Stroke, Vec2};
use project::Project;
use recent::RecentProjects;
use runtime::Runtime;
use std::path::PathBuf;
use std::time::Instant;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Altadra — Rust 2D Engine")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Altadra",
        options,
        Box::new(|cc| Ok(Box::new(Altadra::new(cc)))),
    )
}

struct TextDocument {
    path: PathBuf,
    contents: String,
    dirty: bool,
    preview: bool,
    raw: bool,
}

impl TextDocument {
    fn save(&mut self, root: &std::path::Path) -> Result<(), String> {
        if self.dirty {
            project_files::write_text_file(root, &self.path, &self.contents)?;
            self.dirty = false;
        }
        Ok(())
    }
}

#[derive(Default)]
struct EditorFiles {
    documents: Vec<TextDocument>,
    active: Option<usize>,
}

impl EditorFiles {
    fn open(&mut self, path: PathBuf, contents: String) {
        self.open_with_preview(path, contents, false);
    }

    fn open_with_preview(&mut self, path: PathBuf, contents: String, preview: bool) {
        if let Some(index) = self.documents.iter().position(|file| file.path == path) {
            if !preview {
                self.documents[index].preview = false;
            }
            self.active = Some(index);
            return;
        }
        let document = TextDocument {
            path,
            contents,
            dirty: false,
            preview,
            raw: false,
        };
        let index = if preview
            && let Some(index) = self
                .documents
                .iter()
                .position(|file| file.preview && !file.dirty)
        {
            self.documents[index] = document;
            index
        } else {
            self.documents.push(document);
            self.documents.len() - 1
        };
        self.active = Some(index);
    }

    fn close(&mut self, index: usize) {
        self.documents.remove(index);
        self.active = match self.active {
            _ if self.documents.is_empty() => None,
            Some(active) if active == index => Some(index.min(self.documents.len() - 1)),
            Some(active) if active > index => Some(active - 1),
            other => other,
        };
    }

    fn remove_under(&mut self, path: &std::path::Path) {
        for index in (0..self.documents.len()).rev() {
            if self.documents[index].path.starts_with(path) {
                self.close(index);
            }
        }
    }

    fn remap_paths(&mut self, old: &std::path::Path, new: &std::path::Path) {
        for file in &mut self.documents {
            if let Ok(relative) = file.path.strip_prefix(old) {
                file.path = new.join(relative);
            }
        }
    }

    fn save_all(&mut self, root: &std::path::Path) -> Result<(), String> {
        for file in &mut self.documents {
            file.save(root)?;
        }
        Ok(())
    }

    fn read_document(
        &self,
        root: &std::path::Path,
        path: &std::path::Path,
    ) -> Result<documents::Document, String> {
        if let Some(file) = self.documents.iter().find(|file| file.path == path) {
            return documents::Document::parse(
                path.extension().and_then(|ext| ext.to_str()).unwrap_or(""),
                &file.contents,
            );
        }
        documents::load(root, path)
    }

    fn read_script(
        &self,
        root: &std::path::Path,
        path: &std::path::Path,
    ) -> Result<String, String> {
        if let Some(file) = self.documents.iter().find(|file| file.path == path) {
            return Ok(file.contents.clone());
        }
        project_files::read_text_file(root, path)
    }
}

struct Altadra {
    page: AppPage,
    project: Project,
    recents: RecentProjects,
    recent_error: Option<String>,
    project_name: String,
    projects_parent: Option<PathBuf>,
    file_workspace: Option<tempfile::TempDir>,
    left_panel_tab: LeftPanelTab,
    selected_file: Option<PathBuf>,
    files: EditorFiles,
    unsaved_entries: Vec<PathBuf>,
    file_dialog: Option<FileDialogState>,
    save_dialog: Option<SaveProjectDialog>,
    selected: Option<String>,
    runtime: Option<Runtime>,
    textures: std::collections::HashMap<String, egui::TextureHandle>,
    dirty: bool,
    status: String,
    error: Option<String>,
    last_tick: Instant,
    viewport_focus_id: Option<egui::Id>,
}

impl Altadra {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let project = new_project();
        let selected = project
            .scene
            .sprites
            .first()
            .map(|sprite| sprite.id.clone());
        let (recents, recent_error) = match RecentProjects::load() {
            Ok(recents) => (recents, None),
            Err(error) => (RecentProjects::default(), Some(error)),
        };
        let mut app = Self {
            page: AppPage::Home,
            project,
            recents,
            recent_error,
            project_name: "My 2D Game".into(),
            projects_parent: None,
            file_workspace: None,
            left_panel_tab: LeftPanelTab::Project,
            selected_file: None,
            files: EditorFiles::default(),
            unsaved_entries: Vec::new(),
            file_dialog: None,
            save_dialog: None,
            selected,
            runtime: None,
            textures: Default::default(),
            dirty: false,
            status: "Choose a project to get started.".into(),
            error: None,
            last_tick: Instant::now(),
            viewport_focus_id: None,
        };
        app.refresh_textures(&cc.egui_ctx);
        app
    }

    fn report(&mut self, result: Result<(), String>) {
        match result {
            Ok(()) => self.error = None,
            Err(error) => {
                self.status = error.clone();
                self.error = Some(error);
            }
        }
    }

    fn remember_project(&mut self, path: &std::path::Path) {
        match self.recents.record(path) {
            Ok(()) => self.recent_error = None,
            Err(error) => self.recent_error = Some(error),
        }
    }

    fn begin_save_project_dialog(&mut self, close_after_save: bool) {
        let name = self
            .project
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.project_name.clone());
        let location = self
            .project
            .path
            .as_ref()
            .and_then(|path| path.parent())
            .map(std::path::Path::to_path_buf)
            .or_else(|| self.projects_parent.clone());
        self.save_dialog = Some(SaveProjectDialog {
            name,
            location,
            close_after_save,
            error: None,
        });
    }

    fn save_project_dialog_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.save_dialog.as_mut() else {
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        let mut browse = false;
        egui::Window::new("Save project")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label("Project name");
                let response = ui.text_edit_singleline(&mut dialog.name);
                if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    confirm = true;
                }
                ui.add_space(6.0);
                ui.label("Location");
                ui.horizontal(|ui| {
                    match &dialog.location {
                        Some(location) => ui.monospace(location.display().to_string()),
                        None => ui.weak("No location chosen"),
                    };
                    if ui.button("Browse...").clicked() {
                        browse = true;
                    }
                });
                if let Some(location) = &dialog.location {
                    ui.add_space(6.0);
                    ui.weak(format!(
                        "Project folder: {}",
                        location.join(dialog.name.trim()).display()
                    ));
                }
                if let Some(error) = &dialog.error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        confirm = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });

        if browse {
            let mut picker =
                rfd::FileDialog::new().set_title("Choose where to create the project folder");
            if let Some(location) = &dialog.location {
                picker = picker.set_directory(location);
            }
            if let Some(location) = picker.pick_folder() {
                dialog.location = Some(location);
                dialog.error = None;
            }
        }
        if cancel {
            self.save_dialog = None;
            return;
        }
        if !confirm {
            return;
        }
        let Some(location) = dialog.location.clone() else {
            dialog.error = Some("Choose a location for the project folder.".into());
            return;
        };
        let directory = match project_directory_in(&location, &dialog.name) {
            Ok(directory) => directory,
            Err(error) => {
                dialog.error = Some(error);
                return;
            }
        };
        let close_after_save = dialog.close_after_save;
        let name = dialog.name.trim().to_owned();
        match self.save_to(directory) {
            Ok(()) => {
                self.save_dialog = None;
                self.error = None;
                self.project_name = name;
                self.projects_parent = Some(location);
                if close_after_save {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Err(error) => {
                self.status = error.clone();
                if let Some(dialog) = self.save_dialog.as_mut() {
                    dialog.error = Some(error);
                }
            }
        }
    }

    fn activate_project(&mut self, project: Project, ctx: &egui::Context) {
        self.project = project;
        self.file_workspace = None;
        self.left_panel_tab = LeftPanelTab::Project;
        self.selected_file = self.project.path.clone();
        self.files = EditorFiles::default();
        self.unsaved_entries.clear();
        self.file_dialog = None;
        self.save_dialog = None;
        self.selected = self
            .project
            .scene
            .sprites
            .first()
            .map(|sprite| sprite.id.clone());
        self.dirty = false;
        self.runtime = None;
        self.page = AppPage::Editor;
        self.error = self.project.load_warning.clone();
        self.refresh_textures(ctx);
        if let Some(path) = self.project.path.clone() {
            self.remember_project(&path);
            self.status = format!("Opened {}", path.display());
        }
    }

    fn create_project(&mut self, ctx: &egui::Context) {
        if let Err(error) = project::validate_project_name(&self.project_name) {
            self.error = Some(error);
            return;
        }
        let workspace = match tempfile::Builder::new()
            .prefix("altadra-unsaved-")
            .tempdir()
        {
            Ok(workspace) => workspace,
            Err(error) => {
                self.error = Some(format!(
                    "Could not create an unsaved project workspace: {error}"
                ));
                return;
            }
        };
        self.activate_project(new_project(), ctx);
        self.selected_file = Some(workspace.path().to_path_buf());
        let root = workspace.path().to_path_buf();
        self.file_workspace = Some(workspace);
        self.project.manifest.name = self.project_name.trim().to_owned();
        if let Err(error) = self.project.save(&root) {
            self.error = Some(error);
            return;
        }
        self.project.path = None;
        self.open_project_file(root.join(&self.project.manifest_path), false);
        self.dirty = true;
        self.status = "New project is empty and unsaved. Add content, then save it from the toolbar or when exiting.".into();
    }

    fn file_root_path(&self) -> Option<&std::path::Path> {
        self.file_workspace
            .as_ref()
            .map(tempfile::TempDir::path)
            .or(self.project.path.as_deref())
    }

    fn open_project(&mut self, path: &std::path::Path, ctx: &egui::Context) {
        if self.page == AppPage::Editor && !self.confirm_discard() {
            return;
        }
        let has_manifest = match Project::find_manifest(path) {
            Ok(manifest) => manifest.is_some(),
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let result = if !has_manifest && path.join("scene.json").exists() {
            if rfd::MessageDialog::new().set_title("Upgrade legacy project")
                .set_description("Convert this project into TOML object and level files? Original files will be kept.")
                .set_buttons(rfd::MessageButtons::YesNo).show() != rfd::MessageDialogResult::Yes {
                return;
            }
            project::migrate_legacy(path)
        } else {
            Project::load(path)
        };
        match result {
            Ok(project) => {
                self.activate_project(project, ctx);
                if let Some(root) = self.file_root_path() {
                    self.open_project_file(root.join(&self.project.manifest_path), false);
                }
                if let Some(warning) = &self.project.load_warning {
                    self.error = Some(warning.clone());
                }
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn return_home(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        self.dirty = false;
        self.runtime = None;
        self.file_workspace = None;
        self.selected_file = None;
        self.files = EditorFiles::default();
        self.unsaved_entries.clear();
        self.save_dialog = None;
        self.page = AppPage::Home;
        self.error = None;
        self.status = "Choose a project to get started.".into();
    }

    fn home_page(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(15, 21, 31))
                    .inner_margin(32.0),
            )
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(24.0);
                    ui.label(
                        egui::RichText::new("ALTADRA")
                            .size(16.0)
                            .strong()
                            .color(Color32::from_rgb(115, 189, 255)),
                    );
                    ui.add_space(12.0);
                    ui.heading(egui::RichText::new("Make something playable.").size(38.0).strong());
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Your 2D game studio for Rust and Lua.")
                            .size(16.0)
                            .color(Color32::from_rgb(165, 179, 199)),
                    );
                    ui.add_space(26.0);
                });

                if let Some(error) = &self.recent_error {
                    ui.colored_label(Color32::LIGHT_RED, format!("Recent projects: {error}"));
                }
                if let Some(error) = &self.error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }

                ui.columns(2, |columns| {
                    columns[0].group(|ui| {
                        ui.set_min_height(190.0);
                        ui.heading("New project");
                        ui.add_space(8.0);
                        ui.label("Project name");
                        ui.text_edit_singleline(&mut self.project_name);
                        ui.add_space(6.0);
                        ui.label("Save location (optional)");
                        ui.horizontal(|ui| {
                            let location = self
                                .projects_parent
                                .as_ref()
                                .map(|path| path.display().to_string())
                                .unwrap_or_else(|| "Choose later when saving…".into());
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(location).small().color(Color32::GRAY),
                                )
                                .truncate(),
                            );
                            if ui.button("Browse").clicked()
                                && let Some(parent) = rfd::FileDialog::new()
                                    .set_title("Choose a location for your new project")
                                    .pick_folder()
                            {
                                self.projects_parent = Some(parent);
                            }
                        });
                        ui.add_space(6.0);
                        if ui
                            .button(egui::RichText::new("＋  Create project").strong())
                            .clicked()
                        {
                            self.create_project(ctx);
                        }
                        ui.label(
                            egui::RichText::new("Starts unsaved. Save from the editor or when exiting.")
                                .small()
                                .color(Color32::GRAY),
                        );
                    });
                    columns[1].group(|ui| {
                        ui.set_min_height(190.0);
                        ui.heading("Open a project");
                        ui.add_space(8.0);
                        ui.label(
                            "Open an existing project folder or import a project from another location.",
                        );
                        ui.add_space(14.0);
                        if ui
                            .button(egui::RichText::new("↗  Open / import project…").strong())
                            .clicked()
                            && let Some(path) = rfd::FileDialog::new()
                                .set_title("Open or import an Altadra project folder")
                                .pick_folder()
                        {
                            self.open_project(&path, ctx);
                        }
                        ui.add_space(10.0);
                        ui.label(
                            egui::RichText::new("Choose a folder containing an .altgame project.")
                                .small()
                                .color(Color32::GRAY),
                        );
                    });
                });

                ui.add_space(26.0);
                ui.horizontal(|ui| {
                    ui.heading("Recent projects");
                    if (!self.recents.projects.is_empty() || self.recent_error.is_some())
                        && ui.small_button("Clear list").clicked()
                    {
                        match self.recents.clear() {
                            Ok(()) => self.recent_error = None,
                            Err(error) => self.recent_error = Some(error),
                        }
                    }
                });
                ui.separator();
                if self.recents.projects.is_empty() {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(
                            "Your recent projects will appear here. Create or open one to get started.",
                        )
                        .color(Color32::GRAY),
                    );
                } else {
                    let mut open_path = None;
                    let mut remove_path = None;
                    for recent in self.recents.projects.clone() {
                        ui.horizontal(|ui| {
                            let exists = Project::is_project_folder(&recent.path);
                            let label = if exists {
                                recent.name.as_str()
                            } else {
                                "Missing project folder"
                            };
                            if ui
                                .add_enabled(
                                    exists,
                                    egui::Button::new(label).min_size(Vec2::new(160.0, 30.0)),
                                )
                                .clicked()
                            {
                                open_path = Some(recent.path.clone());
                            }
                            ui.label(
                                egui::RichText::new(recent.path.display().to_string())
                                    .small()
                                    .color(Color32::GRAY),
                            );
                            if ui.small_button("Remove").clicked() {
                                remove_path = Some(recent.path.clone());
                            }
                        });
                    }
                    if let Some(path) = open_path {
                        self.open_project(&path, ctx);
                    }
                    if let Some(path) = remove_path {
                        match self.recents.remove(&path) {
                            Ok(()) => self.recent_error = None,
                            Err(error) => self.recent_error = Some(error),
                        }
                    }
                }
                ui.add_space(18.0);
                ui.label(
                    egui::RichText::new("Rust engine  ·  Lua 5.4  ·  portable projects")
                        .small()
                        .color(Color32::from_rgb(110, 127, 150)),
                );
            });
    }

    fn has_unsaved_changes(&self) -> bool {
        self.dirty
            || self.files.documents.iter().any(|file| file.dirty)
            || !self.unsaved_entries.is_empty()
    }

    fn confirm_discard(&self) -> bool {
        !self.has_unsaved_changes()
            || rfd::MessageDialog::new()
                .set_title("Unsaved changes")
                .set_description("Discard unsaved project and file changes?")
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                == rfd::MessageDialogResult::Yes
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if self.page != AppPage::Editor
            || !(self.has_unsaved_changes() || self.project.path.is_none())
            || !ctx.input(|input| input.viewport().close_requested())
        {
            return;
        }

        let description = if self.project.path.is_some() {
            "Save your most recent project changes before exiting?"
        } else {
            "This project has not been saved yet. Save it before exiting?"
        };
        let choice = rfd::MessageDialog::new()
            .set_title("Save project?")
            .set_description(description)
            .set_level(rfd::MessageLevel::Warning)
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .show();

        match choice {
            rfd::MessageDialogResult::Yes => {
                let Some(directory) = self.project.path.clone() else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    self.begin_save_project_dialog(true);
                    return;
                };
                if let Err(error) = self.save_to(directory) {
                    self.error = Some(error.clone());
                    self.status = error;
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                }
            }
            rfd::MessageDialogResult::No => {}
            rfd::MessageDialogResult::Cancel
            | rfd::MessageDialogResult::Ok
            | rfd::MessageDialogResult::Custom(_) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }
    }

    fn refresh_textures(&mut self, ctx: &egui::Context) {
        let mut refreshed = std::collections::HashMap::new();
        for (name, bytes) in &self.project.assets {
            if let Ok(image) = project::decode_image(bytes) {
                let image = image.to_rgba8();
                let size = [image.width() as usize, image.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                refreshed.insert(
                    name.clone(),
                    ctx.load_texture(name, color, Default::default()),
                );
            }
        }
        self.textures = refreshed;
    }

    fn save_to(&mut self, directory: std::path::PathBuf) -> Result<(), String> {
        self.save_project_snapshot(directory.clone())?;
        self.remember_project(&directory);
        Ok(())
    }

    fn save_project_snapshot(&mut self, directory: PathBuf) -> Result<(), String> {
        let root = self
            .file_root_path()
            .map(PathBuf::from)
            .ok_or_else(|| "No project folder is available.".to_owned())?;
        self.files.save_all(&root)?;
        if let documents::Document::Game(game) =
            self.read_document(&root.join(&self.project.manifest_path))?
        {
            self.project.manifest = game;
        }
        let previous_path = self.project.path.clone();
        let workspace_path = self
            .file_workspace
            .as_ref()
            .map(|workspace| workspace.path().to_path_buf());
        // Imports are already staged; cached images must not recreate renamed files.
        let cached_assets = if self.project.format == project::ProjectFormat::Manifest {
            Some(std::mem::take(&mut self.project.assets))
        } else {
            None
        };
        let result = self.project.save(&directory);
        if let Some(assets) = cached_assets {
            self.project.assets = assets;
        }
        result?;
        let saved_root = self
            .project
            .path
            .clone()
            .ok_or_else(|| "Saved project folder was not recorded.".to_owned())?;
        if let Some(workspace_path) = workspace_path {
            if let Err(error) = project_files::copy_workspace_files(&workspace_path, &saved_root) {
                self.project.path = previous_path;
                return Err(error);
            }
            self.files.remap_paths(&workspace_path, &saved_root);
            if let Some(selected) = self.selected_file.as_mut()
                && let Ok(relative) = selected.strip_prefix(&workspace_path)
            {
                *selected = saved_root.join(relative);
            }
            self.file_workspace = None;
        } else if let Some(previous_path) = previous_path
            && previous_path != saved_root
        {
            if let Err(error) = project_files::copy_workspace_files(&previous_path, &saved_root) {
                self.project.path = Some(previous_path);
                return Err(error);
            }
            self.files.remap_paths(&previous_path, &saved_root);
        }
        self.selected_file = self.project.path.clone();
        self.dirty = false;
        self.unsaved_entries.clear();
        self.status = format!("Saved {}", directory.display());
        Ok(())
    }

    fn save_open_text_file(&mut self) -> Result<(), String> {
        let Some(index) = self.files.active else {
            return Ok(());
        };
        self.save_text_file(index)
    }

    fn save_text_file(&mut self, index: usize) -> Result<(), String> {
        if self.runtime.is_some() {
            return Err("Stop play mode before saving project files.".into());
        }
        let root = self
            .file_root_path()
            .map(PathBuf::from)
            .ok_or_else(|| "No project folder is available.".to_owned())?;
        let file = self
            .files
            .documents
            .get_mut(index)
            .ok_or_else(|| "No text file is open.".to_owned())?;
        file.save(&root)?;
        self.unsaved_entries.retain(|entry| *entry != file.path);
        self.status = format!("Saved {}", file.path.display());
        self.error = None;
        Ok(())
    }

    fn open_project_file(&mut self, path: PathBuf, preview: bool) {
        if self.runtime.is_some() {
            self.status = "Stop play mode before opening project files.".into();
            return;
        }
        if let Some(index) = self
            .files
            .documents
            .iter()
            .position(|file| file.path == path)
        {
            if !preview {
                self.files.documents[index].preview = false;
            }
            self.files.active = Some(index);
            self.selected_file = Some(path);
            return;
        }
        self.selected_file = Some(path.clone());
        let Some(root) = self.file_root_path() else {
            self.status = "Save the project before opening its files.".into();
            return;
        };
        let contents = if documents::is_image_path(&path) {
            Ok(String::new())
        } else {
            project_files::read_text_file(root, &path)
        };
        match contents {
            Ok(contents) => {
                if preview {
                    self.files.open_with_preview(path.clone(), contents, true);
                } else {
                    self.files.open(path.clone(), contents);
                }
                self.error = None;
                self.status = format!("Editing {}", path.display());
            }
            Err(error) => {
                self.status = error.clone();
                self.error = Some(error);
            }
        }
    }

    fn begin_file_dialog(&mut self, action: project_files::EntryAction) {
        let Some(root) = self.file_root_path() else {
            return;
        };
        let target = self
            .selected_file
            .as_deref()
            .filter(|selected| selected.is_dir())
            .or_else(|| {
                self.selected_file
                    .as_deref()
                    .and_then(std::path::Path::parent)
            })
            .unwrap_or(root)
            .to_path_buf();
        let (name, target) = if action == project_files::EntryAction::Rename {
            let Some(selected) = self
                .selected_file
                .as_deref()
                .filter(|selected| *selected != root)
            else {
                return;
            };
            let Some(name) = selected.file_name() else {
                return;
            };
            if selected == root.join("assets") {
                self.error = Some("The assets folder cannot be renamed.".into());
                return;
            }
            if selected == root.join(&self.project.manifest_path) {
                self.error = Some("The project settings filename is managed by Altadra.".into());
                return;
            }
            (name.to_string_lossy().into_owned(), selected.to_path_buf())
        } else {
            (String::new(), target)
        };
        self.file_dialog = Some(FileDialogState {
            action,
            target,
            name,
            focused: false,
            kind: None,
        });
    }

    fn begin_document(&mut self, extension: &'static str) {
        self.begin_file_dialog(project_files::EntryAction::NewFile);
        if let Some(dialog) = self.file_dialog.as_mut() {
            dialog.kind = Some(extension);
        }
    }

    fn finish_inline_create(&mut self, outcome: InlineOutcome, ctx: &egui::Context) {
        let Some(dialog) = self.file_dialog.take() else {
            return;
        };
        if outcome == InlineOutcome::Cancel || dialog.name.trim().is_empty() {
            return;
        }
        if dialog.action == project_files::EntryAction::Rename
            && dialog
                .target
                .file_name()
                .is_some_and(|current| current.to_string_lossy() == dialog.name.trim())
        {
            return;
        }
        self.apply_file_dialog(dialog, ctx);
        if let Some(dialog) = self.file_dialog.as_mut() {
            if outcome == InlineOutcome::CommitOnBlur {
                self.file_dialog = None;
            } else {
                dialog.focused = false;
            }
        }
    }

    fn apply_file_dialog(&mut self, dialog: FileDialogState, ctx: &egui::Context) {
        if self.runtime.is_some() {
            self.error = Some("Stop play mode before changing project files.".into());
            self.file_dialog = Some(dialog);
            return;
        }
        let Some(root) = self.file_root_path().map(std::path::Path::to_path_buf) else {
            return;
        };
        let mut reference_updates = Vec::new();
        let result = match dialog.action {
            project_files::EntryAction::NewFile => {
                if let Some(extension) = dialog.kind {
                    let name = if dialog
                        .name
                        .to_ascii_lowercase()
                        .ends_with(&format!(".{extension}"))
                    {
                        dialog.name.clone()
                    } else {
                        format!("{}.{extension}", dialog.name)
                    };
                    let title = std::path::Path::new(&name)
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or("Untitled")
                        .to_owned();
                    let contents = match extension {
                        "altobject" => documents::Document::Object(documents::ObjectDocument::new(title)).to_toml(),
                        "altscene" => documents::Document::Level(documents::LevelDocument { version: 1, name: title, instances: vec![] }).to_toml(),
                        "lua" => Ok("return function(entity, input, dt)\n    -- Update this instance here.\nend\n".into()),
                        _ => Err("Unsupported document template.".into()),
                    };
                    contents.and_then(|contents| {
                        project_files::create_text_file(&root, &dialog.target, &name, &contents)
                    })
                } else {
                    project_files::create_file(&root, &dialog.target, &dialog.name)
                }
            }
            project_files::EntryAction::NewFolder => {
                project_files::create_folder(&root, &dialog.target, &dialog.name)
            }
            project_files::EntryAction::Rename => {
                let old_path = dialog.target.clone();
                let new_path = old_path.parent().unwrap_or(&root).join(&dialog.name);
                match self.rewritten_references(&old_path, &new_path) {
                    Ok(updates) => reference_updates = updates,
                    Err(error) => {
                        self.error = Some(error);
                        self.file_dialog = Some(dialog);
                        return;
                    }
                }
                let asset_rename = asset_name_for_path(&root, &old_path)
                    .filter(|name| self.project.assets.contains_key(name))
                    .map(|old_name| {
                        let new_name = old_name.rsplit_once('/').map_or_else(
                            || dialog.name.clone(),
                            |(parent, _)| format!("{parent}/{}", dialog.name),
                        );
                        (old_name, new_name)
                    });
                if let Some((_, new_name)) = &asset_rename
                    && let Err(error) = documents::validate_image_reference(new_name)
                {
                    self.error = Some(error);
                    self.file_dialog = Some(dialog);
                    return;
                }
                match project_files::rename_entry(&root, &dialog.target, &dialog.name) {
                    Ok(new_path) => {
                        if self.selected_file.as_deref().is_some_and(|selected| {
                            selected == old_path || selected.starts_with(&old_path)
                        }) {
                            self.selected_file = Some(new_path.clone());
                        }
                        self.files
                            .remap_paths(&old_path, &tree_form_path(&root, new_path.clone()));
                        if let Some((old_name, new_name)) = asset_rename
                            && let Some(bytes) = self.project.assets.remove(&old_name)
                        {
                            for sprite in &mut self.project.scene.sprites {
                                if sprite.asset.as_deref() == Some(&old_name) {
                                    sprite.asset = Some(new_name.clone());
                                }
                            }
                            self.project.assets.insert(new_name, bytes);
                            self.dirty = true;
                            self.refresh_textures(ctx);
                        }
                        Ok(new_path)
                    }
                    Err(error) => Err(error),
                }
            }
        };
        match result {
            Ok(path) => {
                let path = tree_form_path(&root, path);
                for (mut document_path, document) in reference_updates {
                    if let Ok(relative) = document_path.strip_prefix(&dialog.target) {
                        document_path = path.join(relative);
                    }
                    let contents = match document.to_toml() {
                        Ok(contents) => contents,
                        Err(error) => {
                            self.error = Some(error);
                            return;
                        }
                    };
                    let index = if let Some(index) = self
                        .files
                        .documents
                        .iter()
                        .position(|file| file.path == document_path)
                    {
                        index
                    } else {
                        self.files.open(document_path, contents.clone());
                        self.files.documents.len() - 1
                    };
                    self.files.documents[index].contents = contents;
                    self.files.documents[index].dirty = true;
                    self.files.documents[index].preview = false;
                }
                if dialog.action == project_files::EntryAction::Rename {
                    let old_path = &dialog.target;
                    for entry in &mut self.unsaved_entries {
                        if let Ok(relative) = entry.strip_prefix(old_path) {
                            *entry = path.join(relative);
                        }
                    }
                } else {
                    self.unsaved_entries.push(path.clone());
                }
                self.selected_file = Some(path.clone());
                self.error = None;
                if dialog.action == project_files::EntryAction::NewFile {
                    self.open_project_file(path, false);
                } else {
                    self.status = format!("Updated {}", path.display());
                }
            }
            Err(error) => {
                self.error = Some(error);
                self.file_dialog = Some(dialog);
            }
        }
    }

    fn delete_selected_entry(&mut self) {
        if self.runtime.is_some() {
            self.error = Some("Stop play mode before changing project files.".into());
            return;
        }
        let (Some(root), Some(entry)) = (
            self.file_root_path().map(std::path::Path::to_path_buf),
            self.selected_file.clone(),
        ) else {
            return;
        };
        if entry == root || entry == root.join(&self.project.manifest_path) {
            self.error = Some("The project folder and its settings file cannot be removed.".into());
            return;
        }
        let is_directory = entry.is_dir();
        let mut description = if is_directory {
            format!(
                "Remove '{}' and everything inside it?",
                entry.file_name().unwrap_or_default().to_string_lossy()
            )
        } else {
            format!(
                "Remove '{}'?",
                entry.file_name().unwrap_or_default().to_string_lossy()
            )
        };
        let dependencies = (|| {
            let relative = entry
                .strip_prefix(&root)
                .map_err(|error| error.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            let mut dependencies = Vec::new();
            for path in document_editor::project_entries(&root)? {
                if !path.starts_with(&entry)
                    && matches!(
                        path.extension().and_then(|ext| ext.to_str()),
                        Some("altgame" | "altobject" | "altscene")
                    )
                    && self.read_document(&path)?.references_under(&relative)
                {
                    dependencies.push(path.display().to_string());
                }
            }
            Ok::<_, String>(dependencies)
        })();
        match dependencies {
            Ok(dependencies) if !dependencies.is_empty() => description.push_str(&format!(
                "\n\nThis will break references in:\n{}",
                dependencies.join("\n")
            )),
            Ok(_) => {}
            Err(error) => {
                self.error = Some(error);
                return;
            }
        }
        if rfd::MessageDialog::new()
            .set_title("Delete project item")
            .set_description(description)
            .set_level(rfd::MessageLevel::Warning)
            .set_buttons(rfd::MessageButtons::YesNo)
            .show()
            != rfd::MessageDialogResult::Yes
        {
            return;
        }
        if self
            .files
            .documents
            .iter()
            .any(|file| file.dirty && file.path.starts_with(&entry))
        {
            let choice = rfd::MessageDialog::new()
                .set_title("Unsaved file changes")
                .set_description("Save changes to the open files in this item before deleting it?")
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(rfd::MessageButtons::YesNoCancel)
                .show();
            match choice {
                rfd::MessageDialogResult::Yes => {
                    for index in 0..self.files.documents.len() {
                        if self.files.documents[index].path.starts_with(&entry)
                            && let Err(error) = self.save_text_file(index)
                        {
                            self.error = Some(error);
                            return;
                        }
                    }
                }
                rfd::MessageDialogResult::No => {}
                _ => return,
            }
        }
        let removed_assets = self
            .project
            .assets
            .keys()
            .filter(|name| root.join(name).strip_prefix(&entry).is_ok())
            .cloned()
            .collect::<Vec<_>>();
        match project_files::delete_entry(&root, &entry) {
            Ok(()) => {
                for asset in removed_assets {
                    self.project.assets.remove(&asset);
                    for sprite in &mut self.project.scene.sprites {
                        if sprite.asset.as_deref() == Some(&asset) {
                            sprite.asset = None;
                        }
                    }
                    self.dirty = true;
                }
                self.files.remove_under(&entry);
                self.unsaved_entries
                    .retain(|unsaved| !unsaved.starts_with(&entry));
                self.selected_file = Some(root);
                self.status = format!("Removed {}", entry.display());
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn handle_project_delete_key(&mut self, ctx: &egui::Context) {
        let should_delete = self.page == AppPage::Editor
            && self.left_panel_tab == LeftPanelTab::Project
            && self.runtime.is_none()
            && self.file_dialog.is_none()
            && self.save_dialog.is_none()
            && !ctx.wants_keyboard_input()
            && ctx.input(|input| input.key_pressed(egui::Key::Delete));
        if should_delete {
            self.delete_selected_entry();
        }
    }

    fn handle_instance_delete_key(&mut self, ctx: &egui::Context) {
        if self.page != AppPage::Editor
            || self.runtime.is_some()
            || self.file_dialog.is_some()
            || self.save_dialog.is_some()
            || !self
                .viewport_focus_id
                .is_some_and(|id| ctx.memory(|memory| memory.has_focus(id)))
        {
            return;
        }
        let Some(index) = self.files.active else {
            return;
        };
        let file = &self.files.documents[index];
        if file.raw
            || !file
                .path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("altscene"))
        {
            return;
        }
        if !ctx.input_mut(|input| {
            input.focused && input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
        }) {
            return;
        }
        let Some(selected) = self.selected.clone() else {
            return;
        };
        match self.read_document(&self.files.documents[index].path) {
            Ok(documents::Document::Level(mut level)) => {
                let Some(instance) = level
                    .instances
                    .iter()
                    .position(|instance| instance.id == selected)
                else {
                    return;
                };
                level.instances.remove(instance);
                self.update_document(index, &documents::Document::Level(level));
                if self.error.is_none() {
                    self.project
                        .scene
                        .sprites
                        .retain(|sprite| sprite.id != selected);
                    self.selected = None;
                    self.status =
                        "Removed instance from level. The object file is unchanged.".into();
                }
            }
            Err(error) => self.error = Some(error),
            Ok(_) => {}
        }
    }

    fn handle_tree_action(&mut self, action: TreeAction, path: PathBuf) {
        self.selected_file = Some(path);
        match action {
            TreeAction::NewFile => self.begin_file_dialog(project_files::EntryAction::NewFile),
            TreeAction::NewFolder => self.begin_file_dialog(project_files::EntryAction::NewFolder),
            TreeAction::Rename => self.begin_file_dialog(project_files::EntryAction::Rename),
            TreeAction::Delete => self.delete_selected_entry(),
            TreeAction::Object => self.begin_document("altobject"),
            TreeAction::Level => self.begin_document("altscene"),
            TreeAction::Behavior => self.begin_document("lua"),
        }
    }

    fn file_action(&mut self, action: FileAction, ctx: &egui::Context) {
        if self.runtime.is_some() {
            self.error = Some("Stop play mode before changing the project.".into());
            return;
        }
        match action {
            FileAction::New => {
                self.return_home();
            }
            FileAction::Open => {
                if !self.confirm_discard() {
                    return;
                }
                if let Some(directory) = rfd::FileDialog::new()
                    .set_title("Open Altadra project")
                    .pick_folder()
                {
                    self.open_project(&directory, ctx);
                }
            }
            FileAction::Save | FileAction::SaveAs => match self.project.path.clone() {
                Some(directory) if action == FileAction::Save => {
                    let result = self.save_to(directory);
                    self.report(result);
                }
                _ => self.begin_save_project_dialog(false),
            },
            FileAction::Import => {
                if let Some(file) = rfd::FileDialog::new()
                    .add_filter("Images", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
                    .pick_file()
                {
                    match self.import_image_file(&file) {
                        Ok(name) => {
                            if let Some(index) = self.files.active
                                && let Ok(documents::Document::Object(mut object)) =
                                    self.read_document(&self.files.documents[index].path)
                            {
                                object.appearance.image = Some(name.clone());
                                self.update_document(index, &documents::Document::Object(object));
                            }
                            self.refresh_textures(ctx);
                            self.status = format!("Imported {name} into the project.");
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
            }
        }
    }

    fn import_image_file(&mut self, source: &std::path::Path) -> Result<String, String> {
        if self.runtime.is_some() {
            return Err("Stop play mode before changing project files.".into());
        }
        let root = self
            .file_root_path()
            .ok_or_else(|| "No project folder is available.".to_owned())?
            .to_path_buf();
        let target = self
            .selected_file
            .as_deref()
            .filter(|selected| selected.is_dir())
            .or_else(|| {
                self.selected_file
                    .as_deref()
                    .and_then(std::path::Path::parent)
            })
            .unwrap_or(&root);
        let target = project_files::checked_path(&root, target, true)?;
        let target = tree_form_path(&root, target);
        let filename = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "Image filename must be valid UTF-8.".to_owned())?;
        let relative = target
            .join(filename)
            .strip_prefix(&root)
            .map_err(|_| "Import destination must be inside the project.".to_owned())?
            .to_str()
            .ok_or_else(|| "Import path must be valid UTF-8.".to_owned())?
            .replace('\\', "/");
        documents::validate_image_reference(&relative)?;
        let metadata =
            std::fs::metadata(source).map_err(|error| format!("Could not read image: {error}"))?;
        if metadata.len() > project::MAX_PNG_BYTES {
            return Err("Images are limited to 64 MB.".into());
        }
        let bytes =
            std::fs::read(source).map_err(|error| format!("Could not read image: {error}"))?;
        project::decode_image(&bytes)?;
        let destination = root.join(&relative);
        let existed = destination.exists();
        project_files::stage_file(&root, &relative, &bytes)?;
        self.project.import_image(&bytes, &relative)?;
        if !existed {
            self.unsaved_entries.push(destination);
        }
        Ok(relative)
    }

    fn update_play(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f64().min(0.1);
        self.last_tick = now;
        let input = play_input(ctx, self.viewport_focus_id);
        let runtime_error = if let Some(runtime) = &mut self.runtime {
            runtime.advance(dt, input).err()
        } else {
            None
        };
        if let Some(error) = runtime_error {
            self.error = Some(error);
            self.runtime = None;
        }
    }

    fn prepare_play(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let root = self
            .file_root_path()
            .map(PathBuf::from)
            .ok_or_else(|| "No project folder is available.".to_owned())?;
        let level = if let Some(file) = self
            .files
            .active
            .and_then(|index| self.files.documents.get(index))
            && file
                .path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("altscene"))
        {
            self.read_document(&file.path)?
        } else {
            let documents::Document::Game(game) =
                self.read_document(&root.join(&self.project.manifest_path))?
            else {
                return Err("Project settings file is invalid.".into());
            };
            let path = game.starting_level.ok_or_else(|| {
                "Choose a starting level in the project settings, or open a level tab.".to_owned()
            })?;
            self.read_document(&root.join(path))?
        };
        let documents::Document::Level(level) = level else {
            return Err("The selected document is not a level.".into());
        };
        self.resolve_level(&level, ctx)
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.add(egui::Button::new(egui::RichText::new("ALTADRA").heading())
                    .frame(false)).on_hover_text("Home").clicked() {
                    self.return_home();
                }
                ui.separator();
                let mut file_action = None;
                ui.menu_button("File", |ui| {
                    for (label, action) in [
                        ("New", FileAction::New), ("Open", FileAction::Open),
                        ("Save", FileAction::Save), ("Save As", FileAction::SaveAs),
                    ] {
                        if ui.add_enabled(self.runtime.is_none(), egui::Button::new(label)).clicked() {
                            file_action = Some(action);
                            ui.close_menu();
                        }
                    }
                });
                if let Some(action) = file_action {
                    self.file_action(action, ctx);
                }
                if ui.add_enabled(self.runtime.is_none(), egui::Button::new("Import Image")).clicked() {
                    self.file_action(FileAction::Import, ctx);
                }
                ui.separator();
                if self.runtime.is_none() {
                    if ui.button("▶ Play").clicked() {
                        let result = self.prepare_play(ctx).and_then(|()| Runtime::start(&self.project.scene));
                        match result {
                            Ok(runtime) => {
                                self.runtime = Some(runtime);
                                self.status = "Playing — click the viewport and use WASD or arrows. Lua scripts run on fixed 60 Hz ticks.".into();
                                self.error = None;
                                self.last_tick = Instant::now();
                            }

                            Err(error) => self.error = Some(error),
                        }
                    }
                } else if ui.button("■ Stop").clicked() {
                    self.runtime = None;
                    self.status = "Stopped. Authoring scene and saved state are unchanged.".into();
                    self.error = None;
                }
                ui.label(if self.has_unsaved_changes() {
                    "● Unsaved changes"
                } else {
                    "Saved"
                });
            });
        });
    }

    fn left_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("project_and_scene")
            .resizable(true)
            .default_width(235.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(self.left_panel_tab == LeftPanelTab::Project, "Project")
                        .clicked()
                    {
                        self.left_panel_tab = LeftPanelTab::Project;
                    }
                });
                ui.separator();

                match self.left_panel_tab {
                    LeftPanelTab::Project => {
                        if let Some(root) = self.file_root_path().map(PathBuf::from) {
                            let root_name = if self.project.path.is_none() {
                                self.project_name.trim().to_owned()
                            } else {
                                root.file_name()
                                    .unwrap_or(root.as_os_str())
                                    .to_string_lossy()
                                    .into_owned()
                            };
                            let all_unsaved = self.file_workspace.is_some();
                            let mut dirty_paths = self.unsaved_entries.clone();
                            dirty_paths.extend(
                                self.files
                                    .documents
                                    .iter()
                                    .filter(|file| file.dirty)
                                    .map(|file| file.path.clone()),
                            );
                            if self.dirty {
                                dirty_paths.push(root.join(&self.project.manifest_path));
                            }
                            let project_unsaved = all_unsaved || !dirty_paths.is_empty();
                            let mut create_file = false;
                            let mut create_folder = false;
                            let mut new_kind = None;
                            let mut context_action = None;
                            ui.horizontal(|ui| {
                                let header = ui
                                    .add(
                                        egui::Label::new(
                                            unsaved_text(&root_name, project_unsaved)
                                                .strong()
                                                .size(13.0),
                                        )
                                        .truncate()
                                        .sense(egui::Sense::click()),
                                    )
                                    .on_hover_text(if project_unsaved {
                                        format!("{} (unsaved changes)", root.display())
                                    } else {
                                        root.display().to_string()
                                    });
                                if header.clicked() {
                                    self.selected_file = Some(root.clone());
                                }
                                response_context_menu(
                                    &header,
                                    &root,
                                    TreeEntryKind::Root,
                                    &mut context_action,
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.menu_button("+  ▾", |ui| {
                                            for (label, extension) in [
                                                ("New Object", "altobject"),
                                                ("New Level", "altscene"),
                                                ("New Behavior", "lua"),
                                            ] {
                                                if ui.button(label).clicked() {
                                                    new_kind = Some(extension);
                                                    ui.close_menu();
                                                }
                                            }
                                            ui.separator();
                                            if ui.button("New File").clicked() {
                                                create_file = true;
                                                ui.close_menu();
                                            }
                                            if ui.button("New Folder").clicked() {
                                                create_folder = true;
                                                ui.close_menu();
                                            }
                                        });
                                    },
                                );
                            });
                            if let Some(extension) = new_kind {
                                self.begin_document(extension);
                            } else if create_file {
                                self.begin_file_dialog(project_files::EntryAction::NewFile);
                            } else if create_folder {
                                self.begin_file_dialog(project_files::EntryAction::NewFolder);
                            }
                            ui.separator();

                            let mut errors = Vec::new();
                            let mut clicked_file = None;
                            let mut inline_outcome = None;
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                let mut tree = TreeContext {
                                    selected_file: &mut self.selected_file,
                                    clicked_file: &mut clicked_file,
                                    context_action: &mut context_action,
                                    errors: &mut errors,
                                    inline_create: &mut self.file_dialog,
                                    inline_outcome: &mut inline_outcome,
                                    all_unsaved,
                                    dirty_paths: &dirty_paths,
                                };
                                render_project_tree(ui, &root, &mut tree);
                                let blank_space = ui.allocate_response(
                                    Vec2::new(
                                        ui.available_width(),
                                        (ui.available_height() - 48.0).max(24.0),
                                    ),
                                    egui::Sense::click(),
                                );
                                // Clicking empty space targets the project folder itself.
                                if blank_space.clicked() || blank_space.secondary_clicked() {
                                    self.selected_file = Some(root.clone());
                                }
                                response_context_menu(
                                    &blank_space,
                                    &root,
                                    TreeEntryKind::Root,
                                    &mut context_action,
                                );
                            });
                            if let Some(outcome) = inline_outcome {
                                self.finish_inline_create(outcome, ctx);
                            }
                            if let Some((action, path)) = context_action {
                                self.handle_tree_action(action, path);
                            }
                            if let Some((path, preview)) = clicked_file {
                                self.open_project_file(path, preview);
                            }
                            for error in errors {
                                ui.colored_label(Color32::LIGHT_RED, error);
                            }
                            if let Some(path) =
                                self.selected_file.as_ref().filter(|path| **path != root)
                            {
                                ui.separator();
                                ui.label(
                                    path.strip_prefix(&root)
                                        .unwrap_or(path)
                                        .display()
                                        .to_string(),
                                )
                                .on_hover_text(path.display().to_string());
                            }
                        } else {
                            ui.label("Create a project to manage its files.");
                        }
                    }
                }
            });
    }

    fn text_editor(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let mut activate = None;
            let mut close = None;
            let mut pin = None;
            egui::ScrollArea::horizontal()
                .id_salt("file_tabs")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (index, file) in self.files.documents.iter().enumerate() {
                            ui.push_id(&file.path, |ui| {
                                ui.group(|ui| {
                                    ui.horizontal(|ui| {
                                        let name = file
                                            .path
                                            .file_name()
                                            .unwrap_or(file.path.as_os_str())
                                            .to_string_lossy();
                                        let unsaved = file.dirty
                                            || self.unsaved_entries.contains(&file.path)
                                            || self.file_workspace.is_some();
                                        let mut label = unsaved_text(&name, unsaved);
                                        if file.preview {
                                            label = label.italics();
                                        }
                                        let response = ui
                                            .selectable_label(
                                                self.files.active == Some(index),
                                                label,
                                            )
                                            .on_hover_text(file.path.display().to_string());
                                        if response.clicked() {
                                            activate = Some(index);
                                        }
                                        if response.double_clicked() {
                                            pin = Some(index);
                                        }
                                        if ui
                                            .small_button("x")
                                            .on_hover_text("Close file")
                                            .clicked()
                                        {
                                            close = Some(index);
                                        }
                                    });
                                });
                            });
                        }
                    });
                });
            if let Some(index) = pin {
                self.files.documents[index].preview = false;
            }
            if let Some(index) = activate {
                self.files.active = Some(index);
                self.selected_file = Some(self.files.documents[index].path.clone());
            }
            if let Some(index) = close {
                self.close_text_file(index);
            }
            let Some(index) = self.files.active else {
                return;
            };
            let path = self.files.documents[index].path.clone();
            if self.runtime.is_some() {
                self.viewport_ui(ui);
                return;
            }
            ui.separator();
            ui.horizontal(|ui| {
                let display_path = self
                    .file_root_path()
                    .and_then(|root| path.strip_prefix(root).ok())
                    .unwrap_or(&path);
                ui.heading(display_path.display().to_string());
                let file_unsaved =
                    self.files.documents[index].dirty || self.unsaved_entries.contains(&path);
                if file_unsaved {
                    ui.label(egui::RichText::new("● Unsaved").color(Color32::YELLOW));
                }
                if ui
                    .add_enabled(file_unsaved, egui::Button::new("Save file"))
                    .clicked()
                    && let Err(error) = self.save_open_text_file()
                {
                    self.error = Some(error);
                }
            });
            ui.separator();
            if matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("altgame" | "altobject" | "altscene")
            ) {
                let raw = &mut self.files.documents[index].raw;
                ui.checkbox(raw, "Edit TOML source");
                if !*raw {
                    match documents::Document::parse(
                        path.extension().and_then(|ext| ext.to_str()).unwrap_or(""),
                        &self.files.documents[index].contents,
                    ) {
                        Ok(documents::Document::Level(level)) => {
                            match self.resolve_level(&level, ctx) {
                                Ok(()) => self.viewport_ui(ui),
                                Err(error) => {
                                    ui.colored_label(Color32::LIGHT_RED, &error);
                                    self.error = Some(error);
                                }
                            }
                        }
                        Ok(documents::Document::Object(object)) => {
                            ui.heading(&object.name);
                            ui.label(
                                "Edit object defaults and choose artwork/behavior in Properties.",
                            );
                            if let Some(image) = &object.appearance.image {
                                self.image_preview(ui, &path, Some(image), ctx);
                            } else {
                                let (rect, _) = ui.allocate_exact_size(
                                    Vec2::new(
                                        object.appearance.width as f32,
                                        object.appearance.height as f32,
                                    )
                                    .min(ui.available_size()),
                                    Sense::hover(),
                                );
                                ui.painter().rect_filled(
                                    rect,
                                    0.0,
                                    Color32::from_rgba_unmultiplied(
                                        object.appearance.tint[0],
                                        object.appearance.tint[1],
                                        object.appearance.tint[2],
                                        object.appearance.tint[3],
                                    ),
                                );
                            }
                        }
                        Ok(documents::Document::Game(game)) => {
                            ui.heading(&game.name);
                            ui.label("Choose the starting level in Properties, then use Play.");
                        }
                        Err(error) => {
                            ui.colored_label(Color32::LIGHT_RED, &error);
                            ui.label("Enable Edit TOML source to repair this document.");
                        }
                    }
                    return;
                }
            }
            if documents::is_image_path(&path) {
                self.image_preview(ui, &path, None, ctx);
                return;
            }
            let response = ui.add(
                egui::TextEdit::multiline(&mut self.files.documents[index].contents)
                    .id_source(("file_editor", &path))
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(24),
            );
            if response.changed() {
                self.files.documents[index].dirty = true;
                self.files.documents[index].preview = false;
                self.error = None;
            }
        });
    }

    fn close_text_file(&mut self, index: usize) {
        if self.files.documents[index].dirty {
            let choice = rfd::MessageDialog::new()
                .set_title("Unsaved file changes")
                .set_description("Save changes to this file before closing it?")
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(rfd::MessageButtons::YesNoCancel)
                .show();
            match choice {
                rfd::MessageDialogResult::Yes => {
                    if let Err(error) = self.save_text_file(index) {
                        self.error = Some(error);
                        return;
                    }
                }
                rfd::MessageDialogResult::No => {}
                _ => return,
            }
        }
        let was_active = self.files.active == Some(index);
        self.files.close(index);
        if was_active && let Some(active) = self.files.active {
            self.selected_file = Some(self.files.documents[active].path.clone());
        }
    }

    fn inspector(&mut self, ctx: &egui::Context) {
        if self.runtime.is_some() {
            return;
        }
        let Some(index) = self.files.active else {
            return;
        };
        let file = &self.files.documents[index];
        if file.raw {
            return;
        }
        let extension = file
            .path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");
        let Ok(mut document) = documents::Document::parse(extension, &file.contents) else {
            return;
        };
        let entries = match self.project_file_names() {
            Ok(entries) => entries,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let inherited = if let documents::Document::Level(level) = &document
            && let Some(instance) = level
                .instances
                .iter()
                .find(|instance| self.selected.as_ref() == Some(&instance.id))
            && let Some(root) = self.file_root_path()
            && let Ok(documents::Document::Object(object)) =
                self.read_document(&root.join(&instance.object))
        {
            let mut sprite = scene::Sprite::new(object.name);
            sprite.width = object.appearance.width;
            sprite.height = object.appearance.height;
            sprite.vx = object.motion.vx;
            sprite.vy = object.motion.vy;
            sprite.visible = object.appearance.visible;
            sprite.tint = object.appearance.tint;
            Some(sprite)
        } else {
            None
        };
        let mut changed = false;
        egui::SidePanel::right("document_properties")
            .resizable(true)
            .default_width(300.0)
            .show(ctx, |ui| {
                ui.add_enabled_ui(self.runtime.is_none(), |ui| {
                    changed = document_editor::properties(
                        ui,
                        &mut document,
                        &entries,
                        &mut self.selected,
                        inherited.as_ref(),
                    );
                });
            });
        if changed {
            self.update_document(index, &document);
        }
    }

    fn project_file_names(&self) -> Result<Vec<String>, String> {
        let root = self
            .file_root_path()
            .ok_or_else(|| "No project folder is available.".to_owned())?;
        document_editor::project_entries(root)?
            .into_iter()
            .map(|path| {
                path.strip_prefix(root)
                    .map(|path| path.to_string_lossy().replace('\\', "/"))
                    .map_err(|error| error.to_string())
            })
            .collect()
    }

    fn rewritten_references(
        &self,
        old: &std::path::Path,
        new: &std::path::Path,
    ) -> Result<Vec<(PathBuf, documents::Document)>, String> {
        let root = self
            .file_root_path()
            .ok_or_else(|| "No project folder is available.".to_owned())?;
        let old = old
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let new = new
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let mut updates = Vec::new();
        for path in document_editor::project_entries(root)? {
            if matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("altgame" | "altobject" | "altscene")
            ) {
                let mut document = self.read_document(&path)?;
                if document.rewrite_references(&old, &new) {
                    document.validate()?;
                    updates.push((path, document));
                }
            }
        }
        Ok(updates)
    }

    fn resolve_level(
        &mut self,
        level: &documents::LevelDocument,
        ctx: &egui::Context,
    ) -> Result<(), String> {
        if self.runtime.is_some() {
            return Ok(());
        }
        let root = self
            .file_root_path()
            .ok_or_else(|| "No project folder is available.".to_owned())?;
        let (scene, assets) = Project::resolve_scene(
            root,
            level,
            |path| self.read_document(path),
            |path| self.read_script(path),
        )?;
        self.project.scene = scene;
        let textures_changed = assets
            .iter()
            .any(|(name, bytes)| self.project.assets.get(name) != Some(bytes));
        self.project.assets.extend(assets);
        if textures_changed {
            self.refresh_textures(ctx);
        }
        Ok(())
    }

    fn read_document(&self, path: &std::path::Path) -> Result<documents::Document, String> {
        self.files.read_document(
            self.file_root_path()
                .ok_or_else(|| "No project folder is available.".to_owned())?,
            path,
        )
    }

    fn read_script(&self, path: &std::path::Path) -> Result<String, String> {
        self.files.read_script(
            self.file_root_path()
                .ok_or_else(|| "No project folder is available.".to_owned())?,
            path,
        )
    }

    fn image_preview(
        &mut self,
        ui: &mut egui::Ui,
        path: &std::path::Path,
        reference: Option<&str>,
        ctx: &egui::Context,
    ) {
        let result = (|| {
            let root = self
                .file_root_path()
                .ok_or_else(|| "No project folder is available.".to_owned())?;
            let image_path = reference
                .map(|name| root.join(name))
                .unwrap_or_else(|| path.to_path_buf());
            let bytes = project_files::read_image(root, &image_path)?;
            let image = project::decode_image(&bytes)?;
            let image = image.to_rgba8();
            let texture = ctx.load_texture(
                image_path.display().to_string(),
                egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                ),
                Default::default(),
            );
            Ok(texture)
        })();
        match result {
            Ok(texture) => {
                ui.add(egui::Image::new(&texture).max_size(ui.available_size()));
            }
            Err(error) => {
                ui.colored_label(Color32::LIGHT_RED, &error);
                self.error = Some(error);
            }
        }
    }

    fn update_document(&mut self, index: usize, document: &documents::Document) {
        match document.to_toml() {
            Ok(contents) => {
                let file = &mut self.files.documents[index];
                file.contents = contents;
                file.dirty = true;
                file.preview = false;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn viewport(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| self.viewport_ui(ui));
    }

    fn viewport_ui(&mut self, ui: &mut egui::Ui) {
        ui.label("Viewport · origin at center · +X right · +Y down");
        let available = ui.available_size();
        let (rect, response) = ui.allocate_exact_size(available, Sense::click_and_drag());
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        self.viewport_focus_id = Some(response.id);
        let scene = if let Some(runtime) = &self.runtime {
            &runtime.scene
        } else {
            &self.project.scene
        };
        let scale = (rect.width() / 900.0).min(rect.height() / 650.0).max(0.1);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(19, 26, 37));
        let spacing = 32.0 * scale;
        let center = rect.center();
        let x0 = center.x.rem_euclid(spacing);
        let y0 = center.y.rem_euclid(spacing);
        for x in
            std::iter::successors(Some(x0), |x| Some(x + spacing)).take_while(|x| *x < rect.right())
        {
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0_f32, Color32::from_rgb(36, 47, 63)),
            );
        }
        for y in std::iter::successors(Some(y0), |y| Some(y + spacing))
            .take_while(|y| *y < rect.bottom())
        {
            painter.line_segment(
                [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                Stroke::new(1.0_f32, Color32::from_rgb(36, 47, 63)),
            );
        }
        for sprite in &scene.sprites {
            if !sprite.visible {
                continue;
            }
            let center = Pos2::new(
                center.x + sprite.x as f32 * scale,
                center.y + sprite.y as f32 * scale,
            );
            let size = Vec2::new(
                sprite.width as f32 * sprite.scale_x as f32 * scale,
                sprite.height as f32 * sprite.scale_y as f32 * scale,
            );
            let color = Color32::from_rgba_unmultiplied(
                sprite.tint[0],
                sprite.tint[1],
                sprite.tint[2],
                sprite.tint[3],
            );
            let angle = (sprite.rotation as f32).to_radians();
            let corners = rotated_corners(center, size, angle);
            if let Some(texture) = sprite
                .asset
                .as_ref()
                .and_then(|name| self.textures.get(name))
            {
                let mut mesh = egui::Mesh::with_texture(texture.id());
                mesh.vertices = corners
                    .iter()
                    .zip([
                        Pos2::new(0.0, 0.0),
                        Pos2::new(1.0, 0.0),
                        Pos2::new(1.0, 1.0),
                        Pos2::new(0.0, 1.0),
                    ])
                    .map(|(pos, uv)| egui::epaint::Vertex {
                        pos: *pos,
                        uv,
                        color,
                    })
                    .collect();
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                painter.add(egui::Shape::mesh(mesh));
            } else {
                painter.add(egui::Shape::convex_polygon(
                    corners.to_vec(),
                    color,
                    Stroke::NONE,
                ));
            }
            if self.runtime.is_none() && self.selected.as_deref() == Some(&sprite.id) {
                for i in 0..4 {
                    painter.line_segment(
                        [corners[i], corners[(i + 1) % 4]],
                        Stroke::new(5.0_f32, Color32::from_rgb(8, 12, 18)),
                    );
                }
                for i in 0..4 {
                    painter.line_segment(
                        [corners[i], corners[(i + 1) % 4]],
                        Stroke::new(2.5_f32, Color32::from_rgb(80, 205, 255)),
                    );
                }
            }
        }
        if (response.clicked() || response.drag_started()) && self.runtime.is_none() {
            let pointer = response.interact_pointer_pos().unwrap_or(rect.center());
            let (wx, wy) = (pointer.x - rect.center().x, pointer.y - rect.center().y);
            self.selected = scene
                .sprites
                .iter()
                .rev()
                .find(|s| {
                    s.visible && {
                        let radians = -(s.rotation as f32).to_radians();
                        let dx = wx - s.x as f32 * scale;
                        let dy = wy - s.y as f32 * scale;
                        let local_x =
                            (dx * radians.cos() - dy * radians.sin()) / (s.scale_x as f32 * scale);
                        let local_y =
                            (dx * radians.sin() + dy * radians.cos()) / (s.scale_y as f32 * scale);
                        local_x.abs() <= s.width as f32 / 2.0
                            && local_y.abs() <= s.height as f32 / 2.0
                    }
                })
                .map(|s| s.id.clone());
        }
        if self.runtime.is_none()
            && let Some(index) = self.files.active
            && let Ok(documents::Document::Level(mut level)) =
                self.read_document(&self.files.documents[index].path)
        {
            let mut changed = false;
            if response.dragged()
                && let Some(instance) = level
                    .instances
                    .iter_mut()
                    .find(|instance| self.selected.as_ref() == Some(&instance.id))
            {
                let delta = ui.input(|input| input.pointer.delta());
                instance.x += f64::from(delta.x / scale);
                instance.y += f64::from(delta.y / scale);
                changed = delta != Vec2::ZERO;
            }
            if let Some(path) = response.dnd_release_payload::<PathBuf>()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("altobject"))
                && let Some(root) = self.file_root_path()
                && let Ok(relative) = path.strip_prefix(root)
            {
                let pointer = ui
                    .input(|input| input.pointer.latest_pos())
                    .unwrap_or(rect.center());
                let id = scene::Sprite::new("instance").id;
                level.instances.push(documents::Instance {
                    id: id.clone(),
                    object: relative.to_string_lossy().replace('\\', "/"),
                    x: f64::from((pointer.x - rect.center().x) / scale),
                    y: f64::from((pointer.y - rect.center().y) / scale),
                    rotation: 0.0,
                    scale_x: 1.0,
                    scale_y: 1.0,
                    overrides: documents::Overrides::default(),
                });
                self.selected = Some(id);
                changed = true;
            }
            if changed {
                self.update_document(index, &documents::Document::Level(level));
            }
        }
    }
}

fn rotated_corners(center: Pos2, size: Vec2, angle: f32) -> [Pos2; 4] {
    let half = size / 2.0;
    let (sin, cos) = angle.sin_cos();
    [
        (-half.x, -half.y),
        (half.x, -half.y),
        (half.x, half.y),
        (-half.x, half.y),
    ]
    .map(|(x, y)| Pos2::new(center.x + x * cos - y * sin, center.y + x * sin + y * cos))
}

fn play_input(ctx: &egui::Context, viewport: Option<egui::Id>) -> runtime::Input {
    if !viewport.is_some_and(|id| ctx.memory(|memory| memory.has_focus(id))) {
        return runtime::Input::default();
    }
    ctx.input(|input| {
        if !input.focused || input.modifiers.ctrl || input.modifiers.command || input.modifiers.alt
        {
            return runtime::Input::default();
        }
        runtime::Input {
            left: input.key_down(egui::Key::A) || input.key_down(egui::Key::ArrowLeft),
            right: input.key_down(egui::Key::D) || input.key_down(egui::Key::ArrowRight),
            up: input.key_down(egui::Key::W) || input.key_down(egui::Key::ArrowUp),
            down: input.key_down(egui::Key::S) || input.key_down(egui::Key::ArrowDown),
        }
    })
}

impl eframe::App for Altadra {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_close_request(ctx);
        self.handle_instance_delete_key(ctx);
        self.handle_project_delete_key(ctx);
        if self.page == AppPage::Editor
            && self.save_dialog.is_none()
            && ctx.input_mut(|input| {
                input.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND,
                    egui::Key::S,
                ))
            })
        {
            self.file_action(FileAction::Save, ctx);
        }
        if self.page == AppPage::Editor {
            self.update_play(ctx);
        }
        if self.runtime.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(1.0 / 60.0));
        }
        match self.page {
            AppPage::Home => self.home_page(ctx),
            AppPage::Editor => {
                self.top_bar(ctx);
                self.left_panel(ctx);
                self.inspector(ctx);
                if self.files.active.is_some() {
                    self.text_editor(ctx);
                } else if self.runtime.is_some() {
                    self.viewport(ctx);
                } else {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.heading("Project workspace");
                        ui.label("Open a file, or use + to create an Object, Level, or Behavior.");
                    });
                }
                self.save_project_dialog_window(ctx);
                egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
                    if let Some(error) = &self.error {
                        ui.colored_label(Color32::LIGHT_RED, error);
                    } else {
                        ui.label(&self.status);
                    }
                });
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppPage {
    Home,
    Editor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LeftPanelTab {
    Project,
}

fn render_project_tree(ui: &mut egui::Ui, directory: &std::path::Path, tree: &mut TreeContext<'_>) {
    render_inline_create(ui, directory, tree);
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            tree.errors
                .push(format!("Could not read {}: {error}", directory.display()));
            return;
        }
    };
    let mut entries = entries
        .filter_map(|entry| match entry {
            Ok(entry) => match entry.file_type() {
                Ok(file_type) if !file_type.is_symlink() => Some(Ok((entry, file_type))),
                Ok(_) => None,
                Err(error) => Some(Err(format!(
                    "Could not inspect an entry in {}: {error}",
                    directory.display()
                ))),
            },
            Err(error) => Some(Err(format!(
                "Could not read an entry in {}: {error}",
                directory.display()
            ))),
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| match (a, b) {
        (Ok((a, a_type)), Ok((b, b_type))) => {
            b_type.is_dir().cmp(&a_type.is_dir()).then_with(|| {
                a.file_name()
                    .to_string_lossy()
                    .to_lowercase()
                    .cmp(&b.file_name().to_string_lossy().to_lowercase())
            })
        }
        _ => std::cmp::Ordering::Equal,
    });

    for entry in entries {
        let (entry, file_type) = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tree.errors.push(error);
                continue;
            }
        };
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let renaming = inline_rename_target(tree.inline_create) == Some(path.as_path());
        if file_type.is_dir() {
            let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                ui.make_persistent_id(&path),
                false,
            );
            if inline_create_target(tree.inline_create)
                .is_some_and(|target| target.starts_with(&path))
            {
                state.set_open(true);
            }
            let (_, header_label, _) = state
                .show_header(ui, |ui| {
                    if renaming && let Some(dialog) = tree.inline_create.as_mut() {
                        ui.label("📁");
                        return inline_name_edit(ui, dialog, "Folder name", tree.inline_outcome);
                    }
                    let response = ui.selectable_label(
                        tree.selected_file.as_ref() == Some(&path),
                        unsaved_text(&format!("📁 {name}"), tree.is_unsaved(&path)),
                    );
                    if response.clicked() {
                        *tree.selected_file = Some(path.clone());
                    }
                    response
                })
                .body(|ui| render_project_tree(ui, &path, tree));
            if !renaming {
                response_context_menu(
                    &header_label.inner,
                    &path,
                    TreeEntryKind::Folder,
                    tree.context_action,
                );
            }
        } else if renaming && let Some(dialog) = tree.inline_create.as_mut() {
            ui.horizontal(|ui| {
                ui.label("📄");
                inline_name_edit(ui, dialog, "File name", tree.inline_outcome);
            });
        } else {
            let response = project_file_row(
                ui,
                &path,
                &name,
                tree.selected_file.as_ref() == Some(&path),
                tree.is_unsaved(&path),
            );
            if response.clicked() {
                *tree.selected_file = Some(path.clone());
                *tree.clicked_file = Some((path.clone(), !response.double_clicked()));
            }
            response_context_menu(&response, &path, TreeEntryKind::File, tree.context_action);
        }
    }
}

fn project_file_row(
    ui: &mut egui::Ui,
    path: &std::path::Path,
    name: &str,
    selected: bool,
    unsaved: bool,
) -> egui::Response {
    let response = ui
        .push_id(path, |ui| {
            ui.add(
                egui::Button::new(unsaved_text(&format!("📄 {name}"), unsaved))
                    .selected(selected)
                    .frame(false)
                    .sense(Sense::click_and_drag()),
            )
        })
        .inner;
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("altobject"))
    {
        response.dnd_set_drag_payload(path.to_path_buf());
    }
    response
}

struct TreeContext<'a> {
    selected_file: &'a mut Option<PathBuf>,
    clicked_file: &'a mut Option<(PathBuf, bool)>,
    context_action: &'a mut Option<(TreeAction, PathBuf)>,
    errors: &'a mut Vec<String>,
    inline_create: &'a mut Option<FileDialogState>,
    inline_outcome: &'a mut Option<InlineOutcome>,
    all_unsaved: bool,
    dirty_paths: &'a [PathBuf],
}

impl TreeContext<'_> {
    fn is_unsaved(&self, path: &std::path::Path) -> bool {
        self.all_unsaved || self.dirty_paths.iter().any(|dirty| dirty.starts_with(path))
    }
}

fn unsaved_text(name: &str, unsaved: bool) -> egui::RichText {
    if unsaved {
        egui::RichText::new(format!("{name} *")).color(Color32::from_rgb(240, 200, 70))
    } else {
        egui::RichText::new(name)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InlineOutcome {
    Commit,
    CommitOnBlur,
    Cancel,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TreeEntryKind {
    Root,
    Folder,
    File,
}

fn inline_create_target(dialog: &Option<FileDialogState>) -> Option<&std::path::Path> {
    dialog
        .as_ref()
        .filter(|dialog| dialog.action != project_files::EntryAction::Rename)
        .map(|dialog| dialog.target.as_path())
}

fn inline_rename_target(dialog: &Option<FileDialogState>) -> Option<&std::path::Path> {
    dialog
        .as_ref()
        .filter(|dialog| dialog.action == project_files::EntryAction::Rename)
        .map(|dialog| dialog.target.as_path())
}

fn render_inline_create(
    ui: &mut egui::Ui,
    directory: &std::path::Path,
    tree: &mut TreeContext<'_>,
) {
    let Some(dialog) = tree
        .inline_create
        .as_mut()
        .filter(|dialog| dialog.action != project_files::EntryAction::Rename)
    else {
        return;
    };
    if dialog.target != directory {
        return;
    }
    let (icon, hint) = if dialog.action == project_files::EntryAction::NewFolder {
        ("📁", "Folder name")
    } else {
        ("📄", "File name")
    };
    ui.horizontal(|ui| {
        ui.label(icon);
        inline_name_edit(ui, dialog, hint, tree.inline_outcome);
    });
}

fn inline_name_edit(
    ui: &mut egui::Ui,
    dialog: &mut FileDialogState,
    hint: &str,
    outcome: &mut Option<InlineOutcome>,
) -> egui::Response {
    let mut output = egui::TextEdit::singleline(&mut dialog.name)
        .hint_text(hint)
        .desired_width(f32::INFINITY)
        .show(ui);
    if !dialog.focused {
        output.response.request_focus();
        // Preselect the name (without its extension) so typing replaces it.
        let end = match dialog.name.rfind('.') {
            Some(dot) if dot > 0 && dialog.action == project_files::EntryAction::Rename => {
                dialog.name[..dot].chars().count()
            }
            _ => dialog.name.chars().count(),
        };
        output
            .state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(end),
            )));
        output.state.store(ui.ctx(), output.response.id);
        dialog.focused = true;
    }
    if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        *outcome = Some(InlineOutcome::Cancel);
    } else if output.response.lost_focus() {
        *outcome = Some(if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            InlineOutcome::Commit
        } else {
            InlineOutcome::CommitOnBlur
        });
    }
    output.response
}

fn response_context_menu(
    response: &egui::Response,
    path: &std::path::Path,
    kind: TreeEntryKind,
    context_action: &mut Option<(TreeAction, PathBuf)>,
) {
    response.context_menu(|ui| {
        if kind != TreeEntryKind::File {
            for (label, action) in [
                ("New Object", TreeAction::Object),
                ("New Level", TreeAction::Level),
                ("New Behavior", TreeAction::Behavior),
            ] {
                if ui.button(label).clicked() {
                    *context_action = Some((action, path.to_path_buf()));
                    ui.close_menu();
                }
            }
            ui.separator();
        }
        if ui.button("New File").clicked() {
            *context_action = Some((TreeAction::NewFile, path.to_path_buf()));
            ui.close_menu();
        }
        if kind != TreeEntryKind::File && ui.button("New Folder").clicked() {
            *context_action = Some((TreeAction::NewFolder, path.to_path_buf()));
            ui.close_menu();
        }
        if kind != TreeEntryKind::Root {
            ui.separator();
            if ui.button("Rename").clicked() {
                *context_action = Some((TreeAction::Rename, path.to_path_buf()));
                ui.close_menu();
            }
            if ui.button("Delete").clicked() {
                *context_action = Some((TreeAction::Delete, path.to_path_buf()));
                ui.close_menu();
            }
        }
    });
}

/// Maps a canonical path (with a `\\?\` prefix on Windows) back onto the project root
/// as shown in the file tree, so paths compare equal to tree entries.
fn tree_form_path(root: &std::path::Path, path: PathBuf) -> PathBuf {
    root.canonicalize()
        .ok()
        .and_then(|canonical| path.strip_prefix(canonical).ok().map(|rel| root.join(rel)))
        .unwrap_or(path)
}

fn asset_name_for_path(root: &std::path::Path, path: &std::path::Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    if !documents::is_image_path(relative) {
        return None;
    }
    Some(relative.to_str()?.replace('\\', "/"))
}

fn new_project() -> Project {
    Project::default()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileAction {
    New,
    Open,
    Save,
    SaveAs,
    Import,
}

#[derive(Clone, Copy)]
enum TreeAction {
    NewFile,
    NewFolder,
    Rename,
    Delete,
    Object,
    Level,
    Behavior,
}

struct FileDialogState {
    action: project_files::EntryAction,
    target: PathBuf,
    name: String,
    focused: bool,
    kind: Option<&'static str>,
}

struct SaveProjectDialog {
    name: String,
    location: Option<PathBuf>,
    close_after_save: bool,
    error: Option<String>,
}

fn project_directory_in(location: &std::path::Path, name: &str) -> Result<PathBuf, String> {
    project::validate_project_name(name)?;
    let directory = location.join(name.trim());
    if directory.exists() {
        return Err(format!(
            "'{}' already exists. Choose another project name or location.",
            directory.display()
        ));
    }
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::{EditorFiles, new_project, project_directory_in, tree_form_path};

    #[test]
    fn delete_in_level_canvas_removes_only_selected_instance() {
        use crate::documents::{Document, Instance, Overrides};
        let mut app = workspace_app();
        create_document(&mut app, "Player", "altobject");
        create_document(&mut app, "Level", "altscene");
        let root = app.file_root_path().unwrap().to_path_buf();
        let object_before = std::fs::read(root.join("Player.altobject")).unwrap();
        let level_path = root.join("Level.altscene");
        let Document::Level(mut level) = app.read_document(&level_path).unwrap() else {
            panic!("Expected level");
        };
        for id in ["first", "second"] {
            level.instances.push(Instance {
                id: id.into(),
                object: "Player.altobject".into(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                scale_x: 1.0,
                scale_y: 1.0,
                overrides: Overrides::default(),
            });
        }
        app.update_document(1, &Document::Level(level));
        app.files.save_all(&root).unwrap();
        let level_before = std::fs::read(&level_path).unwrap();
        let ctx = egui::Context::default();
        app.resolve_level(
            &match app.read_document(&level_path).unwrap() {
                Document::Level(level) => level,
                _ => unreachable!(),
            },
            &ctx,
        )
        .unwrap();
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.viewport(ctx));
        let pos = egui::pos2(100.0, 100.0);
        for pressed in [true, false] {
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                    ..Default::default()
                },
                |ctx| app.viewport(ctx),
            );
        }
        assert!(ctx.memory(|memory| memory.has_focus(app.viewport_focus_id.unwrap())));
        app.selected = Some("first".into());
        let delete = || egui::Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        let _ = ctx.run(
            egui::RawInput {
                events: vec![delete()],
                ..Default::default()
            },
            |ctx| {
                app.handle_instance_delete_key(ctx);
                assert!(!ctx.input(|input| input.key_pressed(egui::Key::Delete)));
                app.viewport(ctx);
            },
        );
        let Document::Level(level) = app.read_document(&level_path).unwrap() else {
            panic!("Expected level");
        };
        assert_eq!(level.instances.len(), 1);
        assert_eq!(level.instances[0].id, "second");
        assert!(app.files.documents[1].dirty);
        assert!(!app.files.documents[1].preview);
        assert!(app.selected.is_none());
        assert_eq!(std::fs::read(&level_path).unwrap(), level_before);
        assert_eq!(
            std::fs::read(root.join("Player.altobject")).unwrap(),
            object_before
        );

        app.selected = Some("second".into());
        let _ = ctx.run(
            egui::RawInput {
                events: vec![delete()],
                ..Default::default()
            },
            |ctx| {
                ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("text-editor")));
                app.handle_instance_delete_key(ctx);
            },
        );
        assert!(
            app.selected.is_some(),
            "Text editing must not delete instances"
        );
        app.runtime = Some(crate::runtime::Runtime::start(&app.project.scene).unwrap());
        let _ = ctx.run(
            egui::RawInput {
                events: vec![delete()],
                ..Default::default()
            },
            |ctx| {
                ctx.memory_mut(|memory| memory.request_focus(app.viewport_focus_id.unwrap()));
                app.handle_instance_delete_key(ctx);
            },
        );
        assert!(
            app.selected.is_some(),
            "Play must not change the authoring level"
        );
    }

    #[test]
    fn focused_play_viewport_delivers_movement_without_leaking_other_focus() {
        let mut app = workspace_app();
        let mut sprite = crate::scene::Sprite::new("Player");
        sprite.script = r"return function(entity, input, dt)
            local speed = 180
            entity.x = entity.x + input.x * speed * dt
            entity.y = entity.y + input.y * speed * dt
        end"
        .into();
        app.project.scene.sprites.push(sprite);
        app.runtime = Some(crate::runtime::Runtime::start(&app.project.scene).unwrap());
        let ctx = egui::Context::default();
        let pos = egui::pos2(100.0, 100.0);
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.viewport(ctx));
        for pressed in [true, false] {
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                    ..Default::default()
                },
                |ctx| app.viewport(ctx),
            );
        }
        assert!(ctx.memory(|memory| memory.has_focus(app.viewport_focus_id.unwrap())));
        assert!(
            ctx.wants_keyboard_input(),
            "This used to block the focused viewport"
        );
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::D,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
                ..Default::default()
            },
            |ctx| {
                let input = super::play_input(ctx, app.viewport_focus_id);
                assert!(input.right);
                app.runtime
                    .as_mut()
                    .unwrap()
                    .advance(1.0 / 60.0, input)
                    .unwrap();
                ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("other-editor")));
                assert!(!super::play_input(ctx, app.viewport_focus_id).right);
                app.viewport(ctx);
            },
        );
        assert_eq!(app.runtime.as_ref().unwrap().scene.sprites[0].x, 3.0);
        assert_eq!(app.project.scene.sprites[0].x, 0.0);
        let _ = ctx.run(
            egui::RawInput {
                focused: false,
                ..Default::default()
            },
            |ctx| {
                ctx.memory_mut(|memory| memory.request_focus(app.viewport_focus_id.unwrap()));
                assert!(!super::play_input(ctx, app.viewport_focus_id).right);
            },
        );
        let _ = ctx.run(
            egui::RawInput {
                modifiers: egui::Modifiers::CTRL,
                ..Default::default()
            },
            |ctx| {
                ctx.memory_mut(|memory| memory.request_focus(app.viewport_focus_id.unwrap()));
                assert!(!super::play_input(ctx, app.viewport_focus_id).right);
            },
        );
    }

    #[test]
    fn explorer_rows_distinguish_click_double_click_and_object_drag() {
        fn frame(
            ctx: &egui::Context,
            time: f64,
            events: Vec<egui::Event>,
            path: &std::path::Path,
        ) -> egui::Response {
            let mut response = None;
            let _ = ctx.run(
                egui::RawInput {
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        response = Some(super::project_file_row(
                            ui,
                            path,
                            "Player.altobject",
                            false,
                            false,
                        ));
                    });
                },
            );
            response.unwrap()
        }
        fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }
        }
        let ctx = egui::Context::default();
        let path = std::path::Path::new("Player.altobject");
        let pos = frame(&ctx, 0.0, vec![], path).rect.center();
        frame(
            &ctx,
            0.1,
            vec![egui::Event::PointerMoved(pos), button(pos, true)],
            path,
        );
        let click = frame(&ctx, 0.15, vec![button(pos, false)], path);
        assert!(click.clicked());
        assert!(!click.double_clicked());
        assert!(egui::DragAndDrop::payload::<std::path::PathBuf>(&ctx).is_none());
        frame(&ctx, 0.2, vec![button(pos, true)], path);
        let double = frame(&ctx, 0.25, vec![button(pos, false)], path);
        assert!(double.clicked());
        assert!(double.double_clicked());
        frame(&ctx, 1.0, vec![button(pos, true)], path);
        let drag = frame(
            &ctx,
            1.1,
            vec![egui::Event::PointerMoved(pos + egui::vec2(40.0, 0.0))],
            path,
        );
        assert!(drag.drag_started());
        assert!(!drag.clicked());
        assert_eq!(
            egui::DragAndDrop::payload::<std::path::PathBuf>(&ctx)
                .unwrap()
                .as_path(),
            path
        );

        let ctx = egui::Context::default();
        let path = std::path::Path::new("Player.lua");
        let pos = frame(&ctx, 0.0, vec![], path).rect.center();
        frame(
            &ctx,
            0.1,
            vec![egui::Event::PointerMoved(pos), button(pos, true)],
            path,
        );
        assert!(frame(&ctx, 0.2, vec![button(pos, false)], path).clicked());
        assert!(egui::DragAndDrop::payload::<std::path::PathBuf>(&ctx).is_none());
    }

    fn workspace_app() -> super::Altadra {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().to_path_buf();
        let mut project = new_project();
        project.manifest.name = "Test Game".into();
        project.save(&root).unwrap();
        project.path = None;
        super::Altadra {
            page: super::AppPage::Editor,
            project,
            recents: Default::default(),
            recent_error: None,
            project_name: "Test Game".into(),
            projects_parent: None,
            file_workspace: Some(workspace),
            left_panel_tab: super::LeftPanelTab::Project,
            selected_file: Some(root),
            files: EditorFiles::default(),
            unsaved_entries: vec![],
            file_dialog: None,
            save_dialog: None,
            selected: None,
            runtime: None,
            textures: Default::default(),
            dirty: true,
            status: String::new(),
            error: None,
            last_tick: std::time::Instant::now(),
            viewport_focus_id: None,
        }
    }
    #[test]
    fn imports_copy_original_names_into_selected_project_folders() {
        let source_dir = tempfile::tempdir().unwrap();
        let source = source_dir.path().join("Player.png");
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(2, 2))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        std::fs::write(&source, bytes.get_ref()).unwrap();
        let mut app = workspace_app();
        let root = app.file_root_path().unwrap().to_path_buf();
        let folder = crate::project_files::create_folder(&root, &root, "Art").unwrap();
        app.selected_file = Some(super::tree_form_path(&root, folder));
        assert_eq!(app.import_image_file(&source).unwrap(), "Art/Player.png");
        assert_eq!(
            std::fs::read(root.join("Art\\Player.png")).unwrap(),
            *bytes.get_ref()
        );
        assert!(app.unsaved_entries.contains(&root.join("Art\\Player.png")));
        assert!(!root.join("assets").exists());
        app.selected_file = Some(root.join("Art\\Player.png"));
        assert_eq!(app.import_image_file(&source).unwrap(), "Art/Player.png");
        assert_eq!(app.unsaved_entries.len(), 1);
        std::fs::remove_file(&source).unwrap();
        assert!(crate::project_files::read_image(&root, &root.join("Art\\Player.png")).is_ok());
        std::fs::write(&source, bytes.get_ref()).unwrap();
        crate::project_files::stage_file(&root, "Player.png", b"existing").unwrap();
        app.selected_file = None;
        assert!(
            app.import_image_file(&source)
                .unwrap_err()
                .contains("different contents")
        );
        assert_eq!(std::fs::read(root.join("Player.png")).unwrap(), b"existing");
        std::fs::remove_file(root.join("Player.png")).unwrap();
        assert_eq!(app.import_image_file(&source).unwrap(), "Player.png");
        app.selected_file = Some(source_dir.path().to_path_buf());
        assert!(app.import_image_file(&source).is_err());
        std::fs::remove_file(&source).unwrap();
        let saved = tempfile::tempdir().unwrap();
        let saved_root = saved.path().join("Game");
        app.save_project_snapshot(saved_root.clone()).unwrap();
        assert_eq!(
            std::fs::read(saved_root.join("Art\\Player.png")).unwrap(),
            *bytes.get_ref()
        );
        assert!(app.unsaved_entries.is_empty());
        let copy = tempfile::tempdir().unwrap();
        let copy_root = copy.path().join("Game Copy");
        app.save_project_snapshot(copy_root.clone()).unwrap();
        assert_eq!(
            std::fs::read(copy_root.join("Art\\Player.png")).unwrap(),
            *bytes.get_ref()
        );
    }

    fn create_document(app: &mut super::Altadra, name: &str, kind: &'static str) {
        app.apply_file_dialog(
            super::FileDialogState {
                action: crate::project_files::EntryAction::NewFile,
                target: app.file_root_path().unwrap().to_path_buf(),
                name: name.into(),
                focused: false,
                kind: Some(kind),
            },
            &egui::Context::default(),
        );
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    #[test]
    fn custom_creation_first_save_reopen_and_save_as_work_end_to_end() {
        use crate::documents::{Document, Instance, Overrides};
        let mut app = workspace_app();
        create_document(&mut app, "Player", "altobject");
        create_document(&mut app, "Player", "lua");
        create_document(&mut app, "Level1", "altscene");
        assert_eq!(app.unsaved_entries.len(), 3);
        let Document::Object(mut object) = app.read_document(&app.files.documents[0].path).unwrap()
        else {
            panic!("Expected object");
        };
        object.behavior.script = Some("Player.lua".into());
        app.update_document(0, &Document::Object(object));
        app.files.documents[1].contents =
            "return function(entity, input, dt) entity.x = 777 end".into();
        app.files.documents[1].dirty = true;
        let Document::Level(mut level) = app.read_document(&app.files.documents[2].path).unwrap()
        else {
            panic!("Expected level");
        };
        level.instances.push(Instance {
            id: "player".into(),
            object: "Player.altobject".into(),
            x: 10.0,
            y: 20.0,
            rotation: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            overrides: Overrides::default(),
        });
        app.update_document(2, &Document::Level(level));
        let mut game = app.project.manifest.clone();
        game.starting_level = Some("Level1.altscene".into());
        app.files.open(
            app.file_root_path()
                .unwrap()
                .join(&app.project.manifest_path),
            Document::Game(game.clone()).to_toml().unwrap(),
        );
        app.update_document(3, &Document::Game(game));
        app.prepare_play(&egui::Context::default()).unwrap();
        let destination = tempfile::tempdir().unwrap();
        let first = destination.path().join("Game");
        app.save_project_snapshot(first.clone()).unwrap();
        assert!(app.file_workspace.is_none());
        assert!(!app.has_unsaved_changes());
        assert!(app.files.documents.iter().all(|file| file.path.exists()));
        assert!(!first.join("scene.json").exists());
        assert!(!first.join("assets").exists());
        let loaded = crate::project::Project::load(&first).unwrap();
        assert_eq!(loaded.scene.sprites.len(), 1);
        let mut runtime = crate::runtime::Runtime::start(&loaded.scene).unwrap();
        runtime
            .advance(1.0 / 60.0, crate::runtime::Input::default())
            .unwrap();
        assert_eq!(runtime.scene.sprites[0].x, 777.0);
        let copy = destination.path().join("Copy");
        app.save_project_snapshot(copy.clone()).unwrap();
        let copy_root = copy.canonicalize().unwrap();
        assert!(
            app.files
                .documents
                .iter()
                .all(|file| file.path.starts_with(&copy_root))
        );
        assert!(crate::project::Project::load(&copy).is_ok());
    }

    #[test]
    fn renaming_behavior_updates_object_buffer_and_persists_on_save() {
        use crate::documents::Document;
        let mut app = workspace_app();
        create_document(&mut app, "Player", "altobject");
        create_document(&mut app, "Player", "lua");
        let Document::Object(mut object) = app.read_document(&app.files.documents[0].path).unwrap()
        else {
            panic!("Expected object");
        };
        object.behavior.script = Some("Player.lua".into());
        app.update_document(0, &Document::Object(object));
        let root = app.file_root_path().unwrap().to_path_buf();
        app.apply_file_dialog(
            super::FileDialogState {
                action: crate::project_files::EntryAction::Rename,
                target: root.join("Player.lua"),
                name: "Movement.lua".into(),
                focused: false,
                kind: None,
            },
            &egui::Context::default(),
        );
        assert!(app.error.is_none(), "{:?}", app.error);
        let Document::Object(object) = app.read_document(&root.join("Player.altobject")).unwrap()
        else {
            panic!("Expected object");
        };
        assert_eq!(object.behavior.script.as_deref(), Some("Movement.lua"));
        assert!(app.files.documents[0].dirty);
        assert!(!root.join("Player.lua").exists());
        assert!(root.join("Movement.lua").exists());
        let destination = tempfile::tempdir().unwrap();
        app.save_project_snapshot(destination.path().join("Game"))
            .unwrap();
        let root = app.file_root_path().unwrap();
        let Document::Object(object) =
            crate::documents::load(root, &root.join("Player.altobject")).unwrap()
        else {
            panic!("Expected object");
        };
        assert_eq!(object.behavior.script.as_deref(), Some("Movement.lua"));
    }

    #[test]
    fn typed_document_tabs_render_without_a_scene_dock() {
        let mut app = workspace_app();
        let ctx = egui::Context::default();
        for (name, kind) in [
            ("Player", "altobject"),
            ("Level", "altscene"),
            ("Player", "lua"),
        ] {
            create_document(&mut app, name, kind);
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                app.top_bar(ctx);
                app.left_panel(ctx);
                app.inspector(ctx);
                app.text_editor(ctx);
            });
            assert!(!output.shapes.is_empty());
            assert!(app.error.is_none(), "{:?}", app.error);
        }
    }

    #[test]
    fn renaming_image_does_not_recreate_the_cached_original_on_save() {
        use crate::documents::Document;
        let mut app = workspace_app();
        create_document(&mut app, "Player", "altobject");
        let root = app.file_root_path().unwrap().to_path_buf();
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(1, 1))
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        crate::project_files::stage_file(&root, "Images/player.png", encoded.get_ref()).unwrap();
        app.project
            .assets
            .insert("Images/player.png".into(), encoded.into_inner());
        let Document::Object(mut object) =
            app.read_document(&root.join("Player.altobject")).unwrap()
        else {
            panic!("Expected object");
        };
        object.appearance.image = Some("Images/player.png".into());
        app.update_document(0, &Document::Object(object));
        app.apply_file_dialog(
            super::FileDialogState {
                action: crate::project_files::EntryAction::Rename,
                target: root.join("Images").join("player.png"),
                name: "hero.png".into(),
                focused: false,
                kind: None,
            },
            &egui::Context::default(),
        );
        assert!(app.error.is_none(), "{:?}", app.error);
        let destination = tempfile::tempdir().unwrap();
        let saved = destination.path().join("Game");
        app.save_project_snapshot(saved.clone()).unwrap();
        assert!(saved.join("Images").join("hero.png").exists());
        assert!(!saved.join("Images").join("player.png").exists());
        let Document::Object(object) =
            crate::documents::load(&saved, &saved.join("Player.altobject")).unwrap()
        else {
            panic!("Expected object");
        };
        assert_eq!(object.appearance.image.as_deref(), Some("Images/hero.png"));
    }

    #[test]
    fn runtime_snapshot_uses_unsaved_object_and_behavior_tabs() {
        use crate::documents::{Document, Instance, LevelDocument, ObjectDocument, Overrides};
        let root = tempfile::tempdir().unwrap();
        let object_path = root.path().join("Player.altobject");
        let script_path = root.path().join("Player.lua");
        let mut object = ObjectDocument::new("Player");
        object.behavior.script = Some("Player.lua".into());
        std::fs::write(
            &object_path,
            Document::Object(object.clone()).to_toml().unwrap(),
        )
        .unwrap();
        std::fs::write(&script_path, "return function(entity, input, dt) end").unwrap();
        let mut files = EditorFiles::default();
        object.appearance.width = 123.0;
        files.open(object_path, Document::Object(object).to_toml().unwrap());
        files.documents[0].dirty = true;
        files.open(
            script_path,
            "return function(entity, input, dt) entity.x = 456 end".into(),
        );
        files.documents[1].dirty = true;
        let level = LevelDocument {
            version: 1,
            name: "Level".into(),
            instances: vec![Instance {
                id: "player".into(),
                object: "Player.altobject".into(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                scale_x: 1.0,
                scale_y: 1.0,
                overrides: Overrides::default(),
            }],
        };
        let scene = crate::documents::resolve_level(
            root.path(),
            &level,
            |path| files.read_document(root.path(), path),
            |path| files.read_script(root.path(), path),
        )
        .unwrap();
        assert_eq!(scene.sprites[0].width, 123.0);
        let mut runtime = crate::runtime::Runtime::start(&scene).unwrap();
        runtime
            .advance(1.0 / 60.0, crate::runtime::Input::default())
            .unwrap();
        assert_eq!(runtime.scene.sprites[0].x, 456.0);
        assert!(files.documents.iter().all(|file| file.dirty));
    }

    #[test]
    fn previews_replace_only_clean_preview_tabs() {
        let mut files = EditorFiles::default();
        files.open("kept.lua".into(), "kept".into());
        files.open_with_preview("first.lua".into(), "first".into(), true);
        files.open_with_preview("second.lua".into(), "second".into(), true);
        assert_eq!(files.documents.len(), 2);
        assert_eq!(files.documents[1].path, std::path::Path::new("second.lua"));
        assert!(files.documents[1].preview);
        files.open("second.lua".into(), "do not reload".into());
        assert!(!files.documents[1].preview);
        assert_eq!(files.documents[1].contents, "second");
        files.open_with_preview("third.lua".into(), "third".into(), true);
        assert_eq!(files.documents.len(), 3);
        files.documents[2].dirty = true;
        files.open_with_preview("fourth.lua".into(), "fourth".into(), true);
        assert_eq!(files.documents.len(), 4);
        assert!(files.documents[2].dirty);
        files.open_with_preview("kept.lua".into(), "do not reload".into(), true);
        assert!(!files.documents[0].preview);
        assert_eq!(files.active, Some(0));
    }

    #[test]
    fn opening_tabs_preserves_edits_and_reuses_existing_tab() {
        let mut files = EditorFiles::default();
        files.open("first.lua".into(), "original".into());
        files.documents[0].contents = "edited".into();
        files.documents[0].dirty = true;
        files.open("second.lua".into(), "second".into());
        files.open("first.lua".into(), "should not replace edits".into());

        assert_eq!(files.documents.len(), 2);
        assert_eq!(files.active, Some(0));
        assert_eq!(files.documents[0].contents, "edited");
        assert!(files.documents[0].dirty);
    }

    #[test]
    fn closing_tabs_keeps_active_tab_valid() {
        let mut files = EditorFiles::default();
        for name in ["first", "second", "third"] {
            files.open(name.into(), name.into());
        }
        files.close(0);
        assert_eq!(files.active, Some(1));
        assert_eq!(files.documents[1].contents, "third");
        files.close(1);
        assert_eq!(files.active, Some(0));
        files.close(0);
        assert_eq!(files.active, None);
        assert!(files.documents.is_empty());

        for name in ["first", "second", "third"] {
            files.open(name.into(), name.into());
        }
        files.active = Some(1);
        files.close(1);
        assert_eq!(files.active, Some(1));
        assert_eq!(files.documents[1].contents, "third");
        files.active = Some(0);
        files.close(1);
        assert_eq!(files.active, Some(0));
        assert_eq!(files.documents[0].contents, "first");
    }

    #[test]
    fn rename_and_delete_update_all_affected_tabs() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("scripts");
        let new = root.path().join("renamed");
        let mut files = EditorFiles::default();
        files.open(old.join("a.lua"), "unsaved".into());
        files.documents[0].dirty = true;
        files.open(old.join("nested").join("b.lua"), "b".into());
        let unrelated = root.path().join("other.lua");
        files.open(unrelated.clone(), "other".into());

        files.remap_paths(&old, &new);
        assert_eq!(files.documents[0].path, new.join("a.lua"));
        assert_eq!(files.documents[1].path, new.join("nested").join("b.lua"));
        assert!(files.documents[0].dirty);
        assert_eq!(files.documents[0].contents, "unsaved");
        files.remove_under(&new);
        assert_eq!(files.documents.len(), 1);
        assert_eq!(files.documents[0].path, unrelated);
        assert_eq!(files.active, Some(0));
    }

    #[test]
    fn project_save_writes_edits_from_inactive_tabs_too() {
        let root = tempfile::tempdir().unwrap();
        let mut files = EditorFiles::default();
        for name in ["first.lua", "second.lua"] {
            let path = root.path().join(name);
            std::fs::write(&path, "").unwrap();
            files.open(path, format!("edited {name}"));
            files.documents.last_mut().unwrap().dirty = true;
        }
        files.save_all(root.path()).unwrap();
        for file in &files.documents {
            assert_eq!(std::fs::read_to_string(&file.path).unwrap(), file.contents);
            assert!(!file.dirty);
        }
        let moved = root.path().join("new-project");
        files.remap_paths(root.path(), &moved);
        assert_eq!(files.documents[0].path, moved.join("first.lua"));
        assert_eq!(files.active, Some(1));
    }

    #[test]
    fn failed_tab_save_preserves_unsaved_buffer() {
        let root = tempfile::tempdir().unwrap();
        let mut files = EditorFiles::default();
        files.open(root.path().join("missing.lua"), "keep my changes".into());
        files.documents[0].dirty = true;
        assert!(files.save_all(root.path()).is_err());
        assert!(files.documents[0].dirty);
        assert_eq!(files.documents[0].contents, "keep my changes");
    }

    #[test]
    fn canonical_paths_map_back_to_tree_paths() {
        let root = tempfile::tempdir().unwrap();
        let canonical = root
            .path()
            .canonicalize()
            .unwrap()
            .join("scripts")
            .join("a.lua");
        assert_eq!(
            tree_form_path(root.path(), canonical),
            root.path().join("scripts").join("a.lua")
        );
    }

    #[test]
    fn project_directory_is_created_inside_selected_location() {
        let location = tempfile::tempdir().unwrap();

        let directory = project_directory_in(location.path(), "  Space Game ").unwrap();

        assert_eq!(directory, location.path().join("Space Game"));
        std::fs::create_dir(&directory).unwrap();
        assert!(project_directory_in(location.path(), "Space Game").is_err());
        assert!(project_directory_in(location.path(), "").is_err());
    }

    #[test]
    fn new_project_starts_without_sample_sprites_or_files() {
        let project = new_project();

        assert!(project.path.is_none());
        assert!(project.scene.sprites.is_empty());
        assert!(project.assets.is_empty());
    }
}
