#![cfg_attr(
    all(windows, not(debug_assertions), not(test)),
    windows_subsystem = "windows"
)]

mod project;
mod project_files;
mod recent;
mod runtime;
mod scene;

use eframe::egui::{self, Color32, Pos2, Sense, Stroke, Vec2};
use project::Project;
use recent::RecentProjects;
use runtime::Runtime;
use scene::{Scene, Sprite};
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
    open_text_file: Option<PathBuf>,
    text_buffer: String,
    text_file_dirty: bool,
    file_dialog: Option<FileDialogState>,
    save_dialog: Option<SaveProjectDialog>,
    selected: Option<String>,
    runtime: Option<Runtime>,
    textures: std::collections::HashMap<String, egui::TextureHandle>,
    dirty: bool,
    status: String,
    error: Option<String>,
    last_tick: Instant,
    viewport_focused: bool,
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
            open_text_file: None,
            text_buffer: String::new(),
            text_file_dirty: false,
            file_dialog: None,
            save_dialog: None,
            selected,
            runtime: None,
            textures: Default::default(),
            dirty: false,
            status: "Choose a project to get started.".into(),
            error: None,
            last_tick: Instant::now(),
            viewport_focused: false,
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
        self.open_text_file = None;
        self.text_buffer.clear();
        self.text_file_dirty = false;
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
        self.error = None;
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
        self.file_workspace = Some(workspace);
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
        match Project::load(path) {
            Ok(project) => self.activate_project(project, ctx),
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
        self.open_text_file = None;
        self.text_buffer.clear();
        self.text_file_dirty = false;
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
                            egui::RichText::new("Choose a folder containing scene.json and assets.")
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
                            let exists = recent.path.join("scene.json").is_file();
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

    fn confirm_discard(&self) -> bool {
        !(self.dirty || self.text_file_dirty)
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
            || !(self.dirty || self.text_file_dirty || self.project.path.is_none())
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
            if let Ok(image) = image::load_from_memory_with_format(bytes, image::ImageFormat::Png) {
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
        self.save_open_text_file()?;
        let previous_path = self.project.path.clone();
        let workspace_path = self
            .file_workspace
            .as_ref()
            .map(|workspace| workspace.path().to_path_buf());
        self.project.save(&directory).map_err(|e| e.to_string())?;
        let saved_root = self
            .project
            .path
            .clone()
            .ok_or_else(|| "Saved project folder was not recorded.".to_owned())?;
        if let Some(workspace_path) = workspace_path {
            project_files::copy_workspace_files(&workspace_path, &saved_root)?;
            if let Some(open_file) = self.open_text_file.as_mut()
                && let Ok(relative) = open_file.strip_prefix(&workspace_path)
            {
                *open_file = saved_root.join(relative);
            }
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
            self.open_text_file = None;
            self.text_buffer.clear();
            self.text_file_dirty = false;
        }
        self.selected_file = self.project.path.clone();
        self.dirty = false;
        self.status = format!("Saved {}", directory.display());
        self.remember_project(&directory);
        Ok(())
    }

    fn save_open_text_file(&mut self) -> Result<(), String> {
        if !self.text_file_dirty {
            return Ok(());
        }
        if self.runtime.is_some() {
            return Err("Stop play mode before saving project files.".into());
        }
        let root = self
            .file_root_path()
            .ok_or_else(|| "Save the project before editing project files.".to_owned())?;
        let file = self
            .open_text_file
            .as_deref()
            .ok_or_else(|| "No text file is open.".to_owned())?;
        project_files::write_text_file(root, file, &self.text_buffer)?;
        self.text_file_dirty = false;
        self.status = format!("Saved {}", file.display());
        self.error = None;
        Ok(())
    }

    fn open_project_file(&mut self, path: PathBuf) {
        if self.runtime.is_some() {
            self.status = "Stop play mode before opening project files.".into();
            return;
        }
        if self.open_text_file.as_ref() == Some(&path) {
            return;
        }
        if self.text_file_dirty {
            let choice = rfd::MessageDialog::new()
                .set_title("Unsaved file changes")
                .set_description("Save changes to the open file before switching?")
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(rfd::MessageButtons::YesNoCancel)
                .show();
            match choice {
                rfd::MessageDialogResult::Yes => {
                    if let Err(error) = self.save_open_text_file() {
                        self.error = Some(error.clone());
                        self.status = error;
                        return;
                    }
                }
                rfd::MessageDialogResult::No => {}
                _ => return,
            }
        }
        self.selected_file = Some(path.clone());
        let Some(root) = self.file_root_path() else {
            self.status = "Save the project before opening its files.".into();
            return;
        };
        match project_files::read_text_file(root, &path) {
            Ok(contents) => {
                self.open_text_file = Some(path.clone());
                self.text_buffer = contents;
                self.text_file_dirty = false;
                self.error = None;
                self.status = format!("Editing {}", path.display());
            }
            Err(error) => {
                self.open_text_file = None;
                self.text_buffer.clear();
                self.text_file_dirty = false;
                self.status = error;
                self.error = None;
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
            (name.to_string_lossy().into_owned(), selected.to_path_buf())
        } else {
            (String::new(), target)
        };
        self.file_dialog = Some(FileDialogState {
            action,
            target,
            name,
            focused: false,
        });
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
        let result = match dialog.action {
            project_files::EntryAction::NewFile => {
                project_files::create_file(&root, &dialog.target, &dialog.name)
            }
            project_files::EntryAction::NewFolder => {
                project_files::create_folder(&root, &dialog.target, &dialog.name)
            }
            project_files::EntryAction::Rename => {
                let old_path = dialog.target.clone();
                let asset_rename = asset_name_for_path(&root, &old_path)
                    .filter(|name| self.project.assets.contains_key(name))
                    .map(|old_name| {
                        let new_name = format!("assets/{}", dialog.name);
                        (old_name, new_name)
                    });
                if let Some((_, new_name)) = &asset_rename
                    && let Err(error) = project::validate_asset_name(new_name)
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
                        if let Some(open_file) = self.open_text_file.as_mut()
                            && let Ok(relative) = open_file.strip_prefix(&old_path)
                        {
                            *open_file = new_path.join(relative);
                        }
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
                self.selected_file = Some(path.clone());
                self.error = None;
                if dialog.action == project_files::EntryAction::NewFile {
                    self.open_project_file(path);
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
        if entry == root {
            self.error = Some("The project folder itself cannot be removed.".into());
            return;
        }
        let is_directory = entry.is_dir();
        let description = if is_directory {
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
        if self.text_file_dirty
            && self
                .open_text_file
                .as_deref()
                .is_some_and(|file| file == entry || file.starts_with(&entry))
        {
            let choice = rfd::MessageDialog::new()
                .set_title("Unsaved file changes")
                .set_description("Save changes to this file before deleting it?")
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(rfd::MessageButtons::YesNoCancel)
                .show();
            match choice {
                rfd::MessageDialogResult::Yes => {
                    if let Err(error) = self.save_open_text_file() {
                        self.error = Some(error);
                        return;
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
                if self
                    .open_text_file
                    .as_deref()
                    .is_some_and(|file| file == entry || file.starts_with(&entry))
                {
                    self.open_text_file = None;
                    self.text_buffer.clear();
                    self.text_file_dirty = false;
                }
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

    fn handle_tree_action(&mut self, action: TreeAction, path: PathBuf) {
        self.selected_file = Some(path);
        match action {
            TreeAction::NewFile => self.begin_file_dialog(project_files::EntryAction::NewFile),
            TreeAction::NewFolder => self.begin_file_dialog(project_files::EntryAction::NewFolder),
            TreeAction::Rename => self.begin_file_dialog(project_files::EntryAction::Rename),
            TreeAction::Delete => self.delete_selected_entry(),
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
                    .add_filter("PNG image", &["png"])
                    .pick_file()
                {
                    match std::fs::read(&file)
                        .map_err(|e| e.to_string())
                        .and_then(|bytes| {
                            let name =
                                self.project.import_png(&bytes).map_err(|e| e.to_string())?;
                            if let Some(workspace) = &self.file_workspace {
                                project_files::stage_asset_file(workspace.path(), &name, &bytes)?;
                            }
                            Ok(name)
                        }) {
                        Ok(name) => {
                            if let Some(sprite) = self.selected_sprite_mut() {
                                sprite.asset = Some(name);
                            } else {
                                let mut sprite = Sprite::new("Imported sprite");
                                sprite.asset = Some(name);
                                self.selected = Some(sprite.id.clone());
                                self.project.scene.sprites.push(sprite);
                            }
                            self.dirty = true;
                            self.refresh_textures(ctx);
                            self.status = format!("Imported {}", file.display());
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
            }
        }
    }

    fn selected_sprite_mut(&mut self) -> Option<&mut Sprite> {
        let id = self.selected.as_deref()?;
        self.project.scene.sprites.iter_mut().find(|s| s.id == id)
    }

    fn selected_scene_mut(&mut self) -> &mut Scene {
        if let Some(runtime) = &mut self.runtime {
            &mut runtime.scene
        } else {
            &mut self.project.scene
        }
    }

    fn update_play(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f64().min(0.1);
        self.last_tick = now;
        let runtime_error = if let Some(runtime) = &mut self.runtime {
            let input = if self.viewport_focused && !ctx.wants_keyboard_input() {
                runtime::Input {
                    left: ctx
                        .input(|i| i.key_down(egui::Key::A) || i.key_down(egui::Key::ArrowLeft)),
                    right: ctx
                        .input(|i| i.key_down(egui::Key::D) || i.key_down(egui::Key::ArrowRight)),
                    up: ctx.input(|i| i.key_down(egui::Key::W) || i.key_down(egui::Key::ArrowUp)),
                    down: ctx
                        .input(|i| i.key_down(egui::Key::S) || i.key_down(egui::Key::ArrowDown)),
                }
            } else {
                runtime::Input::default()
            };
            runtime.advance(dt, input).err()
        } else {
            None
        };
        if let Some(error) = runtime_error {
            self.error = Some(error);
            self.runtime = None;
        }
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("ALTADRA");
                ui.separator();
                if ui.button("⌂ Home").clicked() {
                    self.return_home();
                }
                ui.separator();
                for (label, action) in [
                    ("New", FileAction::New), ("Open", FileAction::Open),
                    ("Save", FileAction::Save), ("Save As", FileAction::SaveAs),
                    ("Import PNG", FileAction::Import),
                ] {
                    if ui.add_enabled(self.runtime.is_none(), egui::Button::new(label)).clicked() {
                        self.file_action(action, ctx);
                    }
                }
                ui.separator();
                if self.runtime.is_none() {
                    if ui.button("▶ Play").clicked() {
                        match Runtime::start(&self.project.scene) {
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
                ui.label(if self.dirty || self.text_file_dirty {
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
                    if ui
                        .selectable_label(self.left_panel_tab == LeftPanelTab::Scene, "Scene")
                        .clicked()
                    {
                        self.left_panel_tab = LeftPanelTab::Scene;
                        self.file_dialog = None;
                    }
                });
                ui.separator();

                match self.left_panel_tab {
                    LeftPanelTab::Project => {
                        if let Some(root) = self.file_root_path().map(PathBuf::from) {
                            let root_name = if self.project.path.is_none() {
                                format!("{} (unsaved)", self.project_name.trim())
                            } else {
                                root.file_name()
                                    .unwrap_or(root.as_os_str())
                                    .to_string_lossy()
                                    .into_owned()
                            };
                            let mut create_file = false;
                            let mut create_folder = false;
                            let mut context_action = None;
                            ui.horizontal(|ui| {
                                let header = ui
                                    .add(
                                        egui::Label::new(
                                            egui::RichText::new(&root_name).strong().size(13.0),
                                        )
                                        .truncate()
                                        .sense(egui::Sense::click()),
                                    )
                                    .on_hover_text(root.display().to_string());
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
                            if create_file {
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
                            if let Some(path) = clicked_file {
                                self.open_project_file(path);
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
                    LeftPanelTab::Scene => {
                        ui.heading("Scene");
                        let sprites = &self.project.scene.sprites;
                        let selected = self.selected.clone();
                        if sprites.is_empty() {
                            ui.label("This scene is empty. Add a sprite to get started.");
                        }
                        for sprite in sprites {
                            let label = format!("▰  {}", sprite.name);
                            if ui
                                .selectable_label(selected.as_deref() == Some(&sprite.id), label)
                                .clicked()
                            {
                                self.selected = Some(sprite.id.clone());
                            }
                        }
                        ui.separator();
                        ui.add_enabled_ui(self.runtime.is_none(), |ui| {
                            if ui.button("+ Add sprite").clicked() {
                                let sprite = Sprite::new(format!(
                                    "Sprite {}",
                                    self.project.scene.sprites.len() + 1
                                ));
                                self.selected = Some(sprite.id.clone());
                                self.project.scene.sprites.push(sprite);
                                self.dirty = true;
                            }
                            if ui.button("Delete selected").clicked()
                                && let Some(id) = &self.selected
                            {
                                self.project.scene.sprites.retain(|s| &s.id != id);
                                self.selected =
                                    self.project.scene.sprites.first().map(|s| s.id.clone());
                                self.dirty = true;
                            }
                        });
                    }
                }
            });
    }

    fn text_editor(&mut self, ctx: &egui::Context) {
        let Some(path) = self.open_text_file.clone() else {
            return;
        };
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                let display_path = self
                    .file_root_path()
                    .and_then(|root| path.strip_prefix(root).ok())
                    .unwrap_or(&path);
                ui.heading(display_path.display().to_string());
                if self.text_file_dirty {
                    ui.label(egui::RichText::new("● Unsaved").color(Color32::YELLOW));
                }
                if ui
                    .add_enabled(self.text_file_dirty, egui::Button::new("Save file"))
                    .clicked()
                    && let Err(error) = self.save_open_text_file()
                {
                    self.error = Some(error);
                }
                if ui.button("Close file").clicked() {
                    self.close_open_text_file();
                }
            });
            ui.separator();
            let response = ui.add(
                egui::TextEdit::multiline(&mut self.text_buffer)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(24),
            );
            if response.changed() {
                self.text_file_dirty = true;
                self.error = None;
            }
        });
    }

    fn close_open_text_file(&mut self) {
        if self.text_file_dirty {
            let choice = rfd::MessageDialog::new()
                .set_title("Unsaved file changes")
                .set_description("Save changes to this file before closing it?")
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(rfd::MessageButtons::YesNoCancel)
                .show();
            match choice {
                rfd::MessageDialogResult::Yes => {
                    if let Err(error) = self.save_open_text_file() {
                        self.error = Some(error);
                        return;
                    }
                }
                rfd::MessageDialogResult::No => {}
                _ => return,
            }
        }
        self.open_text_file = None;
        self.text_buffer.clear();
        self.text_file_dirty = false;
    }

    fn inspector(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("inspector")
            .resizable(true)
            .default_width(280.0)
            .show(ctx, |ui| {
                ui.heading("Properties");
                let Some(id) = self.selected.clone() else {
                    ui.label("Select a sprite in the hierarchy or viewport.");
                    return;
                };
                let editing_enabled = self.runtime.is_none();
                let Some(sprite) = self
                    .selected_scene_mut()
                    .sprites
                    .iter_mut()
                    .find(|s| s.id == id)
                else {
                    ui.label("Selected sprite is not in this scene.");
                    return;
                };
                let mut changed = false;
                ui.add_enabled_ui(editing_enabled, |ui| {
                    changed |= ui.text_edit_singleline(&mut sprite.name).changed();
                    ui.horizontal(|ui| {
                        ui.label("Position");
                        changed |= ui
                            .add(egui::DragValue::new(&mut sprite.x).prefix("X "))
                            .changed();
                        changed |= ui
                            .add(egui::DragValue::new(&mut sprite.y).prefix("Y "))
                            .changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("Size");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut sprite.width)
                                    .range(1.0..=4096.0)
                                    .prefix("W "),
                            )
                            .changed();
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut sprite.height)
                                    .range(1.0..=4096.0)
                                    .prefix("H "),
                            )
                            .changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("Scale");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut sprite.scale_x)
                                    .range(0.01..=100.0)
                                    .prefix("X "),
                            )
                            .changed();
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut sprite.scale_y)
                                    .range(0.01..=100.0)
                                    .prefix("Y "),
                            )
                            .changed();
                    });
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut sprite.rotation, -360.0..=360.0)
                                .text("Rotation °"),
                        )
                        .changed();
                    let mut tint = Color32::from_rgba_unmultiplied(
                        sprite.tint[0],
                        sprite.tint[1],
                        sprite.tint[2],
                        sprite.tint[3],
                    );
                    if ui.color_edit_button_srgba(&mut tint).changed() {
                        sprite.tint = tint.to_array();
                        changed = true;
                    }
                    changed |= ui.checkbox(&mut sprite.visible, "Visible").changed();
                    ui.separator();
                    ui.label("Velocity (units/second)");
                    ui.horizontal(|ui| {
                        changed |= ui
                            .add(egui::DragValue::new(&mut sprite.vx).prefix("X "))
                            .changed();
                        changed |= ui
                            .add(egui::DragValue::new(&mut sprite.vy).prefix("Y "))
                            .changed();
                    });
                    ui.separator();
                    ui.heading("Lua gameplay script");
                    ui.label("Return a function(entity, input, dt). Edit entity.x/y.");
                    changed |= ui
                        .add(
                            egui::TextEdit::multiline(&mut sprite.script)
                                .desired_rows(12)
                                .code_editor()
                                .desired_width(f32::INFINITY),
                        )
                        .changed();
                    if let Some(asset) = &sprite.asset {
                        ui.small(format!("Image: {asset}"));
                    }
                });
                if changed {
                    if editing_enabled {
                        self.dirty = true;
                    } else {
                        self.status = "Play mode changes are temporary.".into();
                    }
                }
            });
    }

    fn viewport(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Viewport · origin at center · +X right · +Y down");
            let available = ui.available_size();
            let (rect, response) = ui.allocate_exact_size(available, Sense::click());
            if response.clicked() && self.runtime.is_some() {
                response.request_focus();
            }
            self.viewport_focused = response.has_focus();
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
            for x in std::iter::successors(Some(x0), |x| Some(x + spacing))
                .take_while(|x| *x < rect.right())
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
            if response.clicked() && self.runtime.is_none() {
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
                            let local_x = (dx * radians.cos() - dy * radians.sin())
                                / (s.scale_x as f32 * scale);
                            let local_y = (dx * radians.sin() + dy * radians.cos())
                                / (s.scale_y as f32 * scale);
                            local_x.abs() <= s.width as f32 / 2.0
                                && local_y.abs() <= s.height as f32 / 2.0
                        }
                    })
                    .map(|s| s.id.clone());
            }
        });
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

impl eframe::App for Altadra {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_close_request(ctx);
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
                if self.open_text_file.is_some() {
                    self.text_editor(ctx);
                } else {
                    self.viewport(ctx);
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
    Scene,
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
                        format!("📁 {name}"),
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
            let response = ui.selectable_label(
                tree.selected_file.as_ref() == Some(&path),
                format!("📄 {name}"),
            );
            if response.clicked() {
                *tree.selected_file = Some(path.clone());
                *tree.clicked_file = Some(path.clone());
            }
            response_context_menu(&response, &path, TreeEntryKind::File, tree.context_action);
        }
    }
}

struct TreeContext<'a> {
    selected_file: &'a mut Option<PathBuf>,
    clicked_file: &'a mut Option<PathBuf>,
    context_action: &'a mut Option<(TreeAction, PathBuf)>,
    errors: &'a mut Vec<String>,
    inline_create: &'a mut Option<FileDialogState>,
    inline_outcome: &'a mut Option<InlineOutcome>,
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

fn asset_name_for_path(root: &std::path::Path, path: &std::path::Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    if relative.parent()? != std::path::Path::new("assets") {
        return None;
    }
    Some(format!(
        "assets/{}",
        relative.file_name()?.to_string_lossy()
    ))
}

fn new_project() -> Project {
    Project {
        scene: Scene {
            version: 1,
            sprites: Vec::new(),
        },
        path: None,
        assets: Default::default(),
    }
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
}

struct FileDialogState {
    action: project_files::EntryAction,
    target: PathBuf,
    name: String,
    focused: bool,
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
    use super::{new_project, project_directory_in};

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
