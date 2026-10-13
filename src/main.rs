#![cfg_attr(
    all(windows, not(debug_assertions), not(test)),
    windows_subsystem = "windows"
)]

mod project;
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
    left_panel_tab: LeftPanelTab,
    selected_file: Option<PathBuf>,
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
            left_panel_tab: LeftPanelTab::Project,
            selected_file: None,
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

    fn new_project_directory(&mut self) -> Option<PathBuf> {
        if let Err(error) = project::validate_project_name(&self.project_name) {
            self.error = Some(error);
            return None;
        }
        let parent = self.projects_parent.clone().or_else(|| {
            rfd::FileDialog::new()
                .set_title("Choose a location for your new project")
                .pick_folder()
        })?;
        self.projects_parent = Some(parent.clone());
        let directory = parent.join(self.project_name.trim());
        if directory.exists() {
            self.error = Some(format!(
                "'{}' already exists. Choose another name or open the existing project.",
                directory.display()
            ));
            return None;
        }
        Some(directory)
    }

    fn activate_project(&mut self, project: Project, ctx: &egui::Context) {
        self.project = project;
        self.left_panel_tab = LeftPanelTab::Project;
        self.selected_file = None;
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
        self.activate_project(new_project(), ctx);
        self.dirty = true;
        self.status = "New project is empty and unsaved. Add content, then save it from the toolbar or when exiting.".into();
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
        !self.dirty
            || rfd::MessageDialog::new()
                .set_title("Unsaved changes")
                .set_description("Discard unsaved scene and script changes?")
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                == rfd::MessageDialogResult::Yes
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if self.page != AppPage::Editor
            || !(self.dirty || self.project.path.is_none())
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
                let directory = if let Some(path) = self.project.path.clone() {
                    Some(path)
                } else {
                    self.new_project_directory()
                };

                let Some(directory) = directory else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    return;
                };
                if self.project.path.is_none() && directory.exists() {
                    self.error = Some(format!(
                        "'{}' already exists. Choose a different project name or save location.",
                        directory.display()
                    ));
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    return;
                }
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
        self.project.save(&directory).map_err(|e| e.to_string())?;
        self.dirty = false;
        self.status = format!("Saved {}", directory.display());
        self.remember_project(&directory);
        Ok(())
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
            FileAction::Save | FileAction::SaveAs => {
                let directory = if action == FileAction::Save {
                    if self.project.path.is_some() {
                        self.project.path.clone()
                    } else {
                        self.new_project_directory()
                    }
                } else {
                    rfd::FileDialog::new()
                        .set_title("Choose a folder for the Altadra project")
                        .pick_folder()
                };
                if let Some(directory) = directory {
                    if directory.join("scene.json").exists()
                        && self.project.path.as_ref() != Some(&directory)
                        && rfd::MessageDialog::new()
                            .set_title("Overwrite project")
                            .set_description("Replace the scene in this project folder?")
                            .set_level(rfd::MessageLevel::Warning)
                            .set_buttons(rfd::MessageButtons::YesNo)
                            .show()
                            != rfd::MessageDialogResult::Yes
                    {
                        return;
                    }
                    let result = self.save_to(directory);
                    self.report(result);
                }
            }
            FileAction::Import => {
                if let Some(file) = rfd::FileDialog::new()
                    .add_filter("PNG image", &["png"])
                    .pick_file()
                {
                    match std::fs::read(&file)
                        .map_err(|e| e.to_string())
                        .and_then(|bytes| {
                            self.project.import_png(&bytes).map_err(|e| e.to_string())
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
                ui.label(if self.dirty { "● Unsaved changes" } else { "Saved" });
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
                    }
                });
                ui.separator();

                match self.left_panel_tab {
                    LeftPanelTab::Project => {
                        if let Some(root) = self.project.path.as_deref() {
                            let root_name = root
                                .file_name()
                                .unwrap_or(root.as_os_str())
                                .to_string_lossy()
                                .into_owned();
                            ui.label(egui::RichText::new(&root_name).strong())
                                .on_hover_text(root.display().to_string());
                            ui.separator();

                            let mut errors = Vec::new();
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                egui::CollapsingHeader::new(format!("📁 {root_name}"))
                                    .id_salt(root)
                                    .default_open(true)
                                    .show(ui, |ui| {
                                        render_project_tree(
                                            ui,
                                            root,
                                            &mut self.selected_file,
                                            &mut errors,
                                        );
                                    });
                            });
                            for error in errors {
                                ui.colored_label(Color32::LIGHT_RED, error);
                            }
                            if let Some(path) = &self.selected_file {
                                ui.separator();
                                ui.label(
                                    path.strip_prefix(root)
                                        .unwrap_or(path)
                                        .display()
                                        .to_string(),
                                )
                                .on_hover_text(path.display().to_string());
                            }
                        } else {
                            ui.label("Unsaved project — no files saved yet.");
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
                            Stroke::new(2.0_f32, Color32::WHITE),
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
                self.viewport(ctx);
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

fn render_project_tree(
    ui: &mut egui::Ui,
    directory: &std::path::Path,
    selected_file: &mut Option<PathBuf>,
    errors: &mut Vec<String>,
) {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(format!("Could not read {}: {error}", directory.display()));
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
                errors.push(error);
                continue;
            }
        };
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if file_type.is_dir() {
            egui::CollapsingHeader::new(format!("📁 {name}"))
                .id_salt(&path)
                .show(ui, |ui| {
                    render_project_tree(ui, &path, selected_file, errors)
                });
        } else if ui
            .selectable_label(selected_file.as_ref() == Some(&path), format!("📄 {name}"))
            .clicked()
        {
            *selected_file = Some(path);
        }
    }
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

#[cfg(test)]
mod tests {
    use super::new_project;

    #[test]
    fn new_project_starts_without_sample_sprites_or_files() {
        let project = new_project();

        assert!(project.path.is_none());
        assert!(project.scene.sprites.is_empty());
        assert!(project.assets.is_empty());
    }
}
