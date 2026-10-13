#![cfg_attr(
    all(windows, not(debug_assertions), not(test)),
    windows_subsystem = "windows"
)]

mod project;
mod runtime;
mod scene;

use eframe::egui::{self, Color32, Pos2, Sense, Stroke, Vec2};
use project::Project;
use runtime::Runtime;
use scene::{Scene, Sprite};
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
    project: Project,
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
        let mut project = Project::default();
        let player = project.scene.sprites[0].id.clone();
        project.scene.sprites[0].script = r#"return function(entity, input, dt)
    local speed = 180
    if input.left then entity.x = entity.x - speed * dt end
    if input.right then entity.x = entity.x + speed * dt end
    if input.up then entity.y = entity.y - speed * dt end
    if input.down then entity.y = entity.y + speed * dt end
end"#
            .into();
        let mut app = Self {
            project,
            selected: Some(player),
            runtime: None,
            textures: Default::default(),
            dirty: true,
            status: "Ready — select a sprite, edit its properties or Lua script, then press Play."
                .into(),
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

    fn handle_close_request(&self, ctx: &egui::Context) {
        if self.dirty
            && ctx.input(|input| input.viewport().close_requested())
            && !self.confirm_discard()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
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
        Ok(())
    }

    fn file_action(&mut self, action: FileAction, ctx: &egui::Context) {
        if self.runtime.is_some() {
            self.error = Some("Stop play mode before changing the project.".into());
            return;
        }
        match action {
            FileAction::New => {
                if !self.confirm_discard() {
                    return;
                }
                self.project = Project::default();
                self.selected = self.project.scene.sprites.first().map(|s| s.id.clone());
                self.dirty = true;
                self.refresh_textures(ctx);
                self.status = "Created a new example scene.".into();
            }
            FileAction::Open => {
                if !self.confirm_discard() {
                    return;
                }
                if let Some(directory) = rfd::FileDialog::new()
                    .set_title("Open Altadra project")
                    .pick_folder()
                {
                    match Project::load(&directory) {
                        Ok(project) => {
                            self.project = project;
                            self.selected =
                                self.project.scene.sprites.first().map(|s| s.id.clone());
                            self.dirty = false;
                            self.refresh_textures(ctx);
                            self.status = format!("Opened {}", directory.display());
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
            }
            FileAction::Save | FileAction::SaveAs => {
                let directory = if action == FileAction::Save {
                    self.project.path.clone()
                } else {
                    None
                };
                let directory = directory.or_else(|| {
                    rfd::FileDialog::new()
                        .set_title("Choose a folder for the Altadra project")
                        .pick_folder()
                });
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

    fn scene_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("scene_hierarchy")
            .resizable(true)
            .default_width(195.0)
            .show(ctx, |ui| {
                ui.heading("Scene");
                let sprites = &self.project.scene.sprites;
                let selected = self.selected.clone();
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
                        let sprite =
                            Sprite::new(format!("Sprite {}", self.project.scene.sprites.len() + 1));
                        self.selected = Some(sprite.id.clone());
                        self.project.scene.sprites.push(sprite);
                        self.dirty = true;
                    }
                    if ui.button("Delete selected").clicked()
                        && let Some(id) = &self.selected
                    {
                        self.project.scene.sprites.retain(|s| &s.id != id);
                        self.selected = self.project.scene.sprites.first().map(|s| s.id.clone());
                        self.dirty = true;
                    }
                });
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
        self.update_play(ctx);
        if self.runtime.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(1.0 / 60.0));
        }
        self.top_bar(ctx);
        self.scene_panel(ctx);
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileAction {
    New,
    Open,
    Save,
    SaveAs,
    Import,
}
