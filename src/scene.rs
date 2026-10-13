use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sprite {
    pub id: String,
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub rotation: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub width: f64,
    pub height: f64,
    pub tint: [u8; 4],
    pub visible: bool,
    pub asset: Option<String>,
    pub vx: f64,
    pub vy: f64,
    #[serde(default)]
    pub script: String,
}

impl Sprite {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: format!("{:016x}", rand_id()),
            name: name.into(),
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            width: 64.0,
            height: 64.0,
            tint: [86, 200, 240, 255],
            visible: true,
            asset: None,
            vx: 0.0,
            vy: 0.0,
            script: String::new(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() || self.id.len() > 64 {
            return Err("Sprite ID is invalid.".into());
        }
        if self.name.trim().is_empty() || self.name.len() > 200 {
            return Err("Sprite names must contain 1–200 characters.".into());
        }
        for value in [
            self.x,
            self.y,
            self.rotation,
            self.scale_x,
            self.scale_y,
            self.width,
            self.height,
            self.vx,
            self.vy,
        ] {
            if !value.is_finite() || value.abs() > 1_000_000.0 {
                return Err(format!("Invalid numeric value on '{}'.", self.name));
            }
        }
        if self.scale_x <= 0.0 || self.scale_y <= 0.0 || self.width <= 0.0 || self.height <= 0.0 {
            return Err(format!(
                "Sprite '{}' needs positive size and scale.",
                self.name
            ));
        }
        if self.script.len() > 64 * 1024 {
            return Err("Lua scripts are limited to 64 KB.".into());
        }
        if let Some(asset) = &self.asset {
            crate::project::validate_asset_name(asset)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub version: u32,
    pub sprites: Vec<Sprite>,
}

impl Default for Scene {
    fn default() -> Self {
        let mut player = Sprite::new("Player · Lua / WASD");
        player.x = -100.0;
        let moving = Sprite {
            x: 100.0,
            y: -100.0,
            tint: [255, 190, 85, 255],
            vy: 30.0,
            ..Sprite::new("Moving sprite")
        };
        let platform = Sprite {
            y: 140.0,
            width: 400.0,
            height: 24.0,
            tint: [116, 207, 145, 255],
            ..Sprite::new("Platform")
        };
        Self {
            version: 1,
            sprites: vec![player, moving, platform],
        }
    }
}

impl Scene {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!("Unsupported scene version {}.", self.version));
        }
        if self.sprites.len() > 10_000 {
            return Err("Scenes are limited to 10,000 sprites.".into());
        }
        let mut ids = std::collections::HashSet::new();
        for sprite in &self.sprites {
            sprite.validate()?;
            if !ids.insert(&sprite.id) {
                return Err(format!("Duplicate sprite ID '{}'.", sprite.id));
            }
        }
        Ok(())
    }
}

fn rand_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
        ^ std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_validation_catches_bad_and_duplicate_sprites() {
        let mut scene = Scene::default();
        scene.validate().unwrap();
        scene.sprites[1].id = scene.sprites[0].id.clone();
        assert!(scene.validate().unwrap_err().contains("Duplicate"));
        scene.sprites[1].id = "new-id".into();
        scene.sprites[1].scale_x = 0.0;
        assert!(scene.validate().unwrap_err().contains("positive"));
    }
}
