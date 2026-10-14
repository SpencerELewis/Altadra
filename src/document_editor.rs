use crate::documents::{Document, Instance, Overrides};
use egui::{Color32, Ui};
use std::path::{Path, PathBuf};

pub fn project_entries(root: &Path) -> Result<Vec<PathBuf>, String> {
    fn visit(root: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
        for entry in std::fs::read_dir(root)
            .map_err(|e| format!("Could not list '{}': {e}", root.display()))?
        {
            let entry = entry.map_err(|e| e.to_string())?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                visit(&entry.path(), paths)?;
            } else if kind.is_file() {
                paths.push(entry.path());
            }
        }
        Ok(())
    }
    let mut paths = Vec::new();
    visit(root, &mut paths)?;
    paths.sort();
    Ok(paths)
}

pub fn reference_picker(
    ui: &mut Ui,
    label: &str,
    value: &mut Option<String>,
    entries: &[String],
    extension: &str,
) -> bool {
    let previous = value.clone();
    egui::ComboBox::from_id_salt(label)
        .selected_text(value.as_deref().unwrap_or("None"))
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, "None");
            for entry in entries.iter().filter(|path| {
                if extension == "image" {
                    crate::documents::is_image_path(Path::new(path))
                } else {
                    path.to_ascii_lowercase().ends_with(extension)
                }
            }) {
                ui.selectable_value(value, Some(entry.clone()), entry);
            }
        });
    previous != *value
}

pub fn properties(
    ui: &mut Ui,
    document: &mut Document,
    entries: &[String],
    selected: &mut Option<String>,
    inherited: Option<&crate::scene::Sprite>,
) -> bool {
    let mut changed = false;
    match document {
        Document::Game(game) => {
            ui.heading("Game settings");
            changed |= ui.text_edit_singleline(&mut game.name).changed();
            ui.label("Starting level");
            changed |= reference_picker(
                ui,
                "Starting level",
                &mut game.starting_level,
                entries,
                ".altscene",
            );
        }
        Document::Object(object) => {
            ui.heading("Object");
            changed |= ui.text_edit_singleline(&mut object.name).changed();
            ui.label("Image");
            changed |=
                reference_picker(ui, "Image", &mut object.appearance.image, entries, "image");
            ui.label("Lua behavior");
            changed |=
                reference_picker(ui, "Behavior", &mut object.behavior.script, entries, ".lua");
            ui.separator();
            ui.horizontal(|ui| {
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut object.appearance.width)
                            .range(1.0..=4096.0)
                            .prefix("Width "),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut object.appearance.height)
                            .range(1.0..=4096.0)
                            .prefix("Height "),
                    )
                    .changed();
            });
            let mut tint = Color32::from_rgba_unmultiplied(
                object.appearance.tint[0],
                object.appearance.tint[1],
                object.appearance.tint[2],
                object.appearance.tint[3],
            );
            if ui.color_edit_button_srgba(&mut tint).changed() {
                object.appearance.tint = tint.to_array();
                changed = true;
            }
            changed |= ui
                .checkbox(&mut object.appearance.visible, "Visible")
                .changed();
            ui.horizontal(|ui| {
                changed |= ui
                    .add(egui::DragValue::new(&mut object.motion.vx).prefix("Velocity X "))
                    .changed();
                changed |= ui
                    .add(egui::DragValue::new(&mut object.motion.vy).prefix("Y "))
                    .changed();
            });
        }
        Document::Level(level) => {
            ui.heading("Level");
            changed |= ui.text_edit_singleline(&mut level.name).changed();
            ui.menu_button("Add object", |ui| {
                for entry in entries.iter().filter(|entry| entry.ends_with(".altobject")) {
                    if ui.button(entry).clicked() {
                        let id = crate::scene::Sprite::new("instance").id;
                        level.instances.push(Instance {
                            id: id.clone(),
                            object: entry.clone(),
                            x: 0.0,
                            y: 0.0,
                            rotation: 0.0,
                            scale_x: 1.0,
                            scale_y: 1.0,
                            overrides: Overrides::default(),
                        });
                        *selected = Some(id);
                        changed = true;
                        ui.close_menu();
                    }
                }
            });
            egui::ScrollArea::vertical()
                .max_height(120.0)
                .show(ui, |ui| {
                    for instance in &level.instances {
                        if ui
                            .selectable_label(
                                selected.as_ref() == Some(&instance.id),
                                format!("{} ({})", instance.object, instance.id),
                            )
                            .clicked()
                        {
                            *selected = Some(instance.id.clone());
                        }
                    }
                });
            let Some(index) = level
                .instances
                .iter()
                .position(|instance| selected.as_ref() == Some(&instance.id))
            else {
                return changed;
            };
            let instance = &mut level.instances[index];
            ui.horizontal(|ui| {
                changed |= ui
                    .add(egui::DragValue::new(&mut instance.x).prefix("X "))
                    .changed();
                changed |= ui
                    .add(egui::DragValue::new(&mut instance.y).prefix("Y "))
                    .changed();
            });
            changed |= ui
                .add(egui::DragValue::new(&mut instance.rotation).prefix("Rotation "))
                .changed();
            ui.horizontal(|ui| {
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut instance.scale_x)
                            .range(0.01..=100.0)
                            .prefix("Scale X "),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut instance.scale_y)
                            .range(0.01..=100.0)
                            .prefix("Y "),
                    )
                    .changed();
            });
            ui.label("Overrides (unchecked values inherit object defaults)");
            for (label, value, default) in [
                (
                    "Width",
                    &mut instance.overrides.width,
                    inherited.map_or(64.0, |sprite| sprite.width),
                ),
                (
                    "Height",
                    &mut instance.overrides.height,
                    inherited.map_or(64.0, |sprite| sprite.height),
                ),
                (
                    "Velocity X",
                    &mut instance.overrides.vx,
                    inherited.map_or(0.0, |sprite| sprite.vx),
                ),
                (
                    "Velocity Y",
                    &mut instance.overrides.vy,
                    inherited.map_or(0.0, |sprite| sprite.vy),
                ),
            ] {
                ui.horizontal(|ui| {
                    let mut enabled = value.is_some();
                    if ui.checkbox(&mut enabled, label).changed() {
                        *value = if enabled { Some(default) } else { None };
                        changed = true;
                    }
                    if let Some(number) = value {
                        changed |= ui.add(egui::DragValue::new(number)).changed();
                    } else {
                        ui.weak(format!("Inherited: {default}"));
                    }
                });
            }
            let mut visible_override = instance.overrides.visible.is_some();
            if ui
                .checkbox(&mut visible_override, "Override visibility")
                .changed()
            {
                instance.overrides.visible =
                    visible_override.then_some(inherited.is_none_or(|sprite| sprite.visible));
                changed = true;
            }
            if let Some(visible) = &mut instance.overrides.visible {
                changed |= ui.checkbox(visible, "Visible").changed();
            }
            let mut tint_override = instance.overrides.tint.is_some();
            if ui.checkbox(&mut tint_override, "Override tint").changed() {
                instance.overrides.tint =
                    tint_override.then_some(inherited.map_or([255; 4], |sprite| sprite.tint));
                changed = true;
            }
            if let Some(tint) = &mut instance.overrides.tint {
                let mut color = Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], tint[3]);
                if ui.color_edit_button_srgba(&mut color).changed() {
                    *tint = color.to_array();
                    changed = true;
                }
            }
            ui.horizontal(|ui| {
                if ui.button("Reset all overrides").clicked() {
                    instance.overrides = Overrides::default();
                    changed = true;
                }
            });
            let mut duplicate = false;
            let mut delete = false;
            ui.horizontal(|ui| {
                duplicate = ui.button("Duplicate").clicked();
                delete = ui.button("Delete instance").clicked();
            });
            if duplicate {
                let mut instance = level.instances[index].clone();
                instance.id = crate::scene::Sprite::new("instance").id;
                instance.x += 32.0;
                *selected = Some(instance.id.clone());
                level.instances.push(instance);
                changed = true;
            } else if delete {
                level.instances.remove(index);
                *selected = None;
                changed = true;
            }
        }
    }
    changed
}
