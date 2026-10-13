use crate::scene::Scene;
use mlua::{Function, Lua, LuaOptions, RegistryKey, StdLib, Table, VmState};
use std::collections::HashMap;

#[derive(Clone, Copy, Default)]
pub struct Input {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
}

pub struct Runtime {
    pub scene: Scene,
    lua: Lua,
    scripts: HashMap<String, RegistryKey>,
    accumulator: f64,
}

impl Runtime {
    pub fn start(scene: &Scene) -> Result<Self, String> {
        scene.validate()?;
        let lua = Lua::new_with(StdLib::NONE, LuaOptions::default()).map_err(|e| e.to_string())?;
        lua.set_memory_limit(64 * 1024 * 1024)
            .map_err(|e| format!("Could not set the Lua memory limit: {e}"))?;
        let mut scripts = HashMap::new();
        for sprite in &scene.sprites {
            if sprite.script.trim().is_empty() {
                continue;
            }
            install_instruction_limit(&lua);
            let result = lua
                .load(&sprite.script)
                .set_name(&sprite.name)
                .eval::<Function>();
            lua.remove_hook();
            let function = result.map_err(|e| format!("Lua script '{}': {e}", sprite.name))?;
            scripts.insert(
                sprite.id.clone(),
                lua.create_registry_value(function)
                    .map_err(|e| e.to_string())?,
            );
        }
        Ok(Self {
            scene: scene.clone(),
            lua,
            scripts,
            accumulator: 0.0,
        })
    }

    pub fn advance(&mut self, elapsed: f64, input: Input) -> Result<(), String> {
        if !elapsed.is_finite() || elapsed < 0.0 {
            return Err("Invalid runtime elapsed time.".into());
        }
        self.accumulator = (self.accumulator + elapsed.min(8.0 / 60.0)).min(8.0 / 60.0);
        let steps = ((self.accumulator + 1e-12) * 60.0).floor() as u32;
        for _ in 0..steps {
            self.step(input)?;
            self.accumulator -= 1.0 / 60.0;
        }
        Ok(())
    }

    fn step(&mut self, input: Input) -> Result<(), String> {
        for sprite in &mut self.scene.sprites {
            sprite.x = (sprite.x + sprite.vx / 60.0).clamp(-1_000_000.0, 1_000_000.0);
            sprite.y = (sprite.y + sprite.vy / 60.0).clamp(-1_000_000.0, 1_000_000.0);
            let Some(key) = self.scripts.get(&sprite.id) else {
                continue;
            };
            let function: Function = self.lua.registry_value(key).map_err(|e| e.to_string())?;
            let entity = self.lua.create_table().map_err(|e| e.to_string())?;
            entity.set("x", sprite.x).map_err(|e| e.to_string())?;
            entity.set("y", sprite.y).map_err(|e| e.to_string())?;
            entity.set("vx", sprite.vx).map_err(|e| e.to_string())?;
            entity.set("vy", sprite.vy).map_err(|e| e.to_string())?;
            entity
                .set("width", sprite.width)
                .map_err(|e| e.to_string())?;
            entity
                .set("height", sprite.height)
                .map_err(|e| e.to_string())?;
            let keys = self.lua.create_table().map_err(|e| e.to_string())?;
            keys.set("left", input.left).map_err(|e| e.to_string())?;
            keys.set("right", input.right).map_err(|e| e.to_string())?;
            keys.set("up", input.up).map_err(|e| e.to_string())?;
            keys.set("down", input.down).map_err(|e| e.to_string())?;
            keys.set("x", (input.right as i8 - input.left as i8) as f64)
                .map_err(|e| e.to_string())?;
            keys.set("y", (input.down as i8 - input.up as i8) as f64)
                .map_err(|e| e.to_string())?;
            install_instruction_limit(&self.lua);
            let result = function.call::<()>((entity.clone(), keys, 1.0 / 60.0));
            self.lua.remove_hook();
            result.map_err(|e| format!("Lua script '{}': {e}", sprite.name))?;
            sprite.x = finite_entity_number(&entity, "x")?;
            sprite.y = finite_entity_number(&entity, "y")?;
            sprite.vx = finite_entity_number(&entity, "vx")?;
            sprite.vy = finite_entity_number(&entity, "vy")?;
            sprite.x = sprite.x.clamp(-1_000_000.0, 1_000_000.0);
            sprite.y = sprite.y.clamp(-1_000_000.0, 1_000_000.0);
        }
        self.scene.validate()
    }
}

fn install_instruction_limit(lua: &Lua) {
    let instruction_checks = std::cell::Cell::new(0);
    lua.set_hook(
        mlua::HookTriggers::new().every_nth_instruction(10_000),
        move |_, _| {
            let checks = instruction_checks.get() + 1;
            instruction_checks.set(checks);
            if checks >= 10 {
                Err(mlua::Error::RuntimeError(
                    "Lua instruction limit exceeded (100,000 instructions per frame).".into(),
                ))
            } else {
                Ok(VmState::Continue)
            }
        },
    );
}

fn finite_entity_number(table: &Table, key: &str) -> Result<f64, String> {
    let value: f64 = table
        .get(key)
        .map_err(|e| format!("Lua entity.{key} must be numeric: {e}"))?;
    if !value.is_finite() {
        return Err(format!("Lua entity.{key} must be finite."));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Sprite;

    #[test]
    fn fixed_step_accumulates_fractional_frames_and_caps_catchup() {
        let scene = Scene {
            sprites: vec![Sprite {
                vx: 60.0,
                ..Sprite::new("moving")
            }],
            ..Default::default()
        };
        let mut runtime = Runtime::start(&scene).unwrap();
        runtime.advance(1.0 / 120.0, Input::default()).unwrap();
        assert_eq!(runtime.scene.sprites[0].x, 0.0);
        runtime.advance(1.0 / 120.0, Input::default()).unwrap();
        assert!((runtime.scene.sprites[0].x - 1.0).abs() < 1e-8);
        runtime.advance(10.0, Input::default()).unwrap();
        assert!((runtime.scene.sprites[0].x - 9.0).abs() < 1e-8);
    }

    #[test]
    fn lua_receives_input_and_updates_runtime_only() {
        let mut scene = Scene::default();
        scene.sprites[0].script = "return function(entity, input, dt) if input.right then entity.x = entity.x + 60 * dt end end".into();
        let mut runtime = Runtime::start(&scene).unwrap();
        runtime
            .advance(
                1.0 / 60.0,
                Input {
                    right: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!((runtime.scene.sprites[0].x - scene.sprites[0].x - 1.0).abs() < 1e-8);
        assert_eq!(scene.sprites[0].x, -100.0);
    }

    #[test]
    fn invalid_or_runaway_script_returns_error() {
        let mut scene = Scene::default();
        scene.sprites[0].script = "return function( end".into();
        assert!(Runtime::start(&scene).err().unwrap().contains("Lua script"));
        scene.sprites[0].script = "return function(entity, input, dt) while true do end end".into();
        let mut runtime = Runtime::start(&scene).unwrap();
        assert!(
            runtime
                .advance(1.0 / 60.0, Input::default())
                .unwrap_err()
                .contains("instruction limit")
        );
        scene.sprites[0].script = "while true do end".into();
        assert!(
            Runtime::start(&scene)
                .err()
                .unwrap()
                .contains("instruction limit")
        );
    }

    #[test]
    fn lua_standard_libraries_are_not_exposed() {
        let mut scene = Scene::default();
        scene.sprites[0].script =
            "return function(entity) entity.x = os == nil and io == nil and debug == nil and 1 or 0 end".into();
        let mut runtime = Runtime::start(&scene).unwrap();
        runtime.advance(1.0 / 60.0, Input::default()).unwrap();
        assert_eq!(runtime.scene.sprites[0].x, 1.0);
    }

    #[test]
    fn lua_memory_usage_is_bounded() {
        let mut scene = Scene::default();
        scene.sprites[0].script =
            "return function(entity) entity.x = string.rep('x', 80 * 1024 * 1024) end".into();
        let mut runtime = Runtime::start(&scene).unwrap();
        assert!(runtime.advance(1.0 / 60.0, Input::default()).is_err());
    }
}
