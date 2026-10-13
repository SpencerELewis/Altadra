use crate::{
    project_files,
    scene::{Scene, Sprite},
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

const VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameDocument {
    pub version: u32,
    pub name: String,
    pub starting_level: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectDocument {
    pub version: u32,
    pub name: String,
    pub appearance: Appearance,
    #[serde(default)]
    pub behavior: Behavior,
    #[serde(default)]
    pub motion: Motion,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Appearance {
    pub image: Option<String>,
    pub width: f64,
    pub height: f64,
    pub tint: [u8; 4],
    pub visible: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Behavior {
    pub script: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Motion {
    pub vx: f64,
    pub vy: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LevelDocument {
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub instances: Vec<Instance>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instance {
    pub id: String,
    pub object: String,
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub rotation: f64,
    #[serde(default = "unit_scale")]
    pub scale_x: f64,
    #[serde(default = "unit_scale")]
    pub scale_y: f64,
    #[serde(default)]
    pub overrides: Overrides,
}

fn unit_scale() -> f64 {
    1.0
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Overrides {
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub tint: Option<[u8; 4]>,
    pub visible: Option<bool>,
    pub vx: Option<f64>,
    pub vy: Option<f64>,
}

#[derive(Clone, Debug)]
pub enum Document {
    Game(GameDocument),
    Object(ObjectDocument),
    Level(LevelDocument),
}

fn validate_header(version: u32, name: &str) -> Result<(), String> {
    if version != VERSION {
        return Err(format!("Unsupported document version {version}."));
    }
    if name.trim().is_empty() || name.len() > 200 {
        return Err("Document names must contain 1 to 200 characters.".into());
    }
    Ok(())
}

fn validate_reference(reference: &str, extension: &str) -> Result<(), String> {
    if reference.is_empty() || reference.contains('\\') {
        return Err(format!(
            "Use a project-relative reference with '/' separators: '{reference}'."
        ));
    }

    for component in reference.split('/') {
        project_files::validate_entry_name(component)?;
    }
    if !reference.to_ascii_lowercase().ends_with(extension) {
        return Err(format!("Reference '{reference}' must end in {extension}."));
    }
    Ok(())
}

pub fn validate_image_reference(reference: &str) -> Result<(), String> {
    validate_reference(reference, ".png")
}

impl ObjectDocument {
    fn resolve(&self, instance: &Instance) -> Result<Sprite, String> {
        let mut sprite = Sprite::new(&self.name);
        sprite.id = instance.id.clone();
        sprite.x = instance.x;
        sprite.y = instance.y;
        sprite.rotation = instance.rotation;
        sprite.scale_x = instance.scale_x;
        sprite.scale_y = instance.scale_y;
        sprite.width = instance.overrides.width.unwrap_or(self.appearance.width);
        sprite.height = instance.overrides.height.unwrap_or(self.appearance.height);
        sprite.tint = instance.overrides.tint.unwrap_or(self.appearance.tint);
        sprite.visible = instance
            .overrides
            .visible
            .unwrap_or(self.appearance.visible);
        sprite.vx = instance.overrides.vx.unwrap_or(self.motion.vx);
        sprite.vy = instance.overrides.vy.unwrap_or(self.motion.vy);
        sprite.asset = self.appearance.image.clone();
        sprite.validate()?;
        Ok(sprite)
    }

    fn validate(&self) -> Result<(), String> {
        validate_header(self.version, &self.name)?;
        if let Some(image) = &self.appearance.image {
            validate_reference(image, ".png")?;
        }
        if let Some(script) = &self.behavior.script {
            validate_reference(script, ".lua")?;
        }
        self.resolve(&Instance {
            id: "validation".into(),
            object: "validation.altobject".into(),
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            overrides: Overrides::default(),
        })?;
        Ok(())
    }
}

impl Document {
    pub fn parse(extension: &str, source: &str) -> Result<Self, String> {
        let document = match extension.to_ascii_lowercase().as_str() {
            "altgame" => Self::Game(
                toml::from_str(source).map_err(|e| format!("Invalid project document: {e}"))?,
            ),
            "altobject" => Self::Object(
                toml::from_str(source).map_err(|e| format!("Invalid object document: {e}"))?,
            ),
            "altscene" => Self::Level(
                toml::from_str(source).map_err(|e| format!("Invalid level document: {e}"))?,
            ),
            _ => return Err(format!("Unsupported document extension '{extension}'.")),
        };
        document.validate()?;
        Ok(document)
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Game(game) => {
                validate_header(game.version, &game.name)?;
                if let Some(level) = &game.starting_level {
                    validate_reference(level, ".altscene")?;
                }
            }
            Self::Object(object) => object.validate()?,
            Self::Level(level) => {
                validate_header(level.version, &level.name)?;
                if level.instances.len() > 10_000 {
                    return Err("Levels are limited to 10,000 instances.".into());
                }
                let mut ids = HashSet::new();
                for instance in &level.instances {
                    validate_reference(&instance.object, ".altobject")?;
                    if !ids.insert(&instance.id) {
                        return Err(format!("Duplicate instance ID '{}'.", instance.id));
                    }
                    ObjectDocument::new("validation").resolve(instance)?;
                }
            }
        }
        Ok(())
    }

    /// Mutable views of every project-relative path this document references.
    fn references_mut(&mut self) -> Vec<&mut String> {
        match self {
            Self::Game(game) => game.starting_level.iter_mut().collect(),
            Self::Object(object) => object
                .appearance
                .image
                .iter_mut()
                .chain(object.behavior.script.iter_mut())
                .collect(),
            Self::Level(level) => level
                .instances
                .iter_mut()
                .map(|instance| &mut instance.object)
                .collect(),
        }
    }

    /// Every project-relative path (with `/` separators) this document references.
    pub fn references(&self) -> Vec<String> {
        self.clone()
            .references_mut()
            .into_iter()
            .map(|value| value.clone())
            .collect()
    }

    /// True when any reference equals `relative` or lies inside it (folder prefix,
    /// matched per component, ASCII case-insensitively like Windows paths).
    pub fn references_under(&self, relative: &str) -> bool {
        self.references()
            .iter()
            .any(|reference| reference_suffix(reference, relative).is_some())
    }

    /// Rewrites references to `old_relative` (a file or folder) so they point at
    /// `new_relative`. Returns whether anything changed.
    pub fn rewrite_references(&mut self, old_relative: &str, new_relative: &str) -> bool {
        let new_relative = normalize_relative(new_relative);
        let mut changed = false;
        for reference in self.references_mut() {
            if let Some(rest) = reference_suffix(reference, old_relative) {
                let updated = format!("{new_relative}{rest}");
                if *reference != updated {
                    *reference = updated;
                    changed = true;
                }
            }
        }
        changed
    }
    pub fn to_toml(&self) -> Result<String, String> {
        self.validate()?;
        match self {
            Self::Game(value) => toml::to_string_pretty(value),
            Self::Object(value) => toml::to_string_pretty(value),
            Self::Level(value) => toml::to_string_pretty(value),
        }
        .map_err(|e| format!("Could not serialize document: {e}"))
    }
}

fn normalize_relative(relative: &str) -> String {
    relative
        .replace('\\', "/")
        .trim_matches('/')
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .collect::<Vec<_>>()
        .join("/")
}

/// If `reference` is `prefix` or inside folder `prefix`, returns the remaining
/// suffix (empty, or starting with `/`).
fn reference_suffix<'a>(reference: &'a str, prefix: &str) -> Option<&'a str> {
    let prefix = normalize_relative(prefix);
    if prefix.is_empty() || reference.len() < prefix.len() {
        return None;
    }
    let (head, rest) = reference.split_at_checked(prefix.len())?;
    (head.eq_ignore_ascii_case(&prefix) && (rest.is_empty() || rest.starts_with('/')))
        .then_some(rest)
}
impl ObjectDocument {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            version: VERSION,
            name: name.into(),
            appearance: Appearance {
                image: None,
                width: 64.0,
                height: 64.0,
                tint: [86, 200, 240, 255],
                visible: true,
            },
            behavior: Behavior::default(),
            motion: Motion::default(),
        }
    }
}

pub fn load(root: &Path, path: &Path) -> Result<Document, String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Document has no extension.".to_owned())?;
    Document::parse(extension, &project_files::read_text_file(root, path)?)
}

pub fn save(root: &Path, path: &Path, document: &Document) -> Result<(), String> {
    let expected = match document {
        Document::Game(_) => "altgame",
        Document::Object(_) => "altobject",
        Document::Level(_) => "altscene",
    };
    if !path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
    {
        return Err(format!(
            "This document must be saved with the .{expected} extension."
        ));
    }
    project_files::write_text_file(root, path, &document.to_toml()?)
}

/// Joins a `/`-separated project reference onto `root` component by component,
/// so it also works with verbatim (`\\?\`) canonical roots on Windows.
pub fn reference_path(root: &Path, reference: &str) -> std::path::PathBuf {
    let mut path = root.to_path_buf();
    for component in reference.split('/').filter(|part| !part.is_empty()) {
        path.push(component);
    }
    path
}
pub fn resolve_level(
    root: &Path,
    level: &LevelDocument,
    mut read_document: impl FnMut(&Path) -> Result<Document, String>,
    mut read_script: impl FnMut(&Path) -> Result<String, String>,
) -> Result<Scene, String> {
    Document::Level(level.clone()).validate()?;
    let mut sprites = Vec::with_capacity(level.instances.len());
    for instance in &level.instances {
        let path = reference_path(root, &instance.object);
        let Document::Object(object) = read_document(&path)? else {
            return Err(format!("'{}' is not an object document.", instance.object));
        };
        object.validate()?;
        let mut sprite = object.resolve(instance)?;
        if let Some(script) = &object.behavior.script {
            sprite.script = read_script(&reference_path(root, script))?;
            if sprite.script.len() > 64 * 1024 {
                return Err("Lua scripts are limited to 64 KB.".into());
            }
        }
        sprites.push(sprite);
    }
    Ok(Scene {
        version: VERSION,
        sprites,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_document_types_round_trip() {
        let documents = [
            (
                "altgame",
                Document::Game(GameDocument {
                    version: 1,
                    name: "Game".into(),
                    starting_level: Some("Levels/Start.altscene".into()),
                }),
            ),
            ("altobject", Document::Object(ObjectDocument::new("Player"))),
            (
                "altscene",
                Document::Level(LevelDocument {
                    version: 1,
                    name: "Level".into(),
                    instances: vec![],
                }),
            ),
        ];
        for (extension, document) in documents {
            let text = document.to_toml().unwrap();
            assert_eq!(
                Document::parse(extension, &text)
                    .unwrap()
                    .to_toml()
                    .unwrap(),
                text
            );
        }
    }

    #[test]
    fn rejects_invalid_versions_fields_and_references() {
        assert!(Document::parse("altgame", "version = 2\nname = 'Game'").is_err());
        assert!(Document::parse("altgame", "version = 1\nname = 'Game'\nunknown = true").is_err());
        for reference in [
            "../bad.lua",
            "/bad.lua",
            "C:/bad.lua",
            "a\\bad.lua",
            "a//bad.lua",
            "wrong.png",
        ] {
            assert!(
                validate_reference(reference, ".lua").is_err(),
                "{reference}"
            );
        }
    }

    fn instance() -> Instance {
        Instance {
            id: "player".into(),
            object: "Player.altobject".into(),
            x: 10.0,
            y: 20.0,
            rotation: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            overrides: Overrides::default(),
        }
    }

    #[test]
    fn resolves_defaults_overrides_and_external_lua() {
        let root = tempfile::tempdir().unwrap();
        let mut object = ObjectDocument::new("Player");
        object.appearance.image = Some("Images/player.png".into());
        object.motion.vx = 20.0;
        object.behavior.script = Some("Player.lua".into());
        let mut placed = instance();
        placed.overrides.width = Some(100.0);
        placed.overrides.visible = Some(false);
        let level = LevelDocument {
            version: 1,
            name: "Level".into(),
            instances: vec![placed],
        };
        let scene = resolve_level(
            root.path(),
            &level,
            |_| Ok(Document::Object(object.clone())),
            |_| Ok("return function(entity, input, dt) entity.x = entity.x + 1 end".into()),
        )
        .unwrap();
        assert_eq!(scene.sprites[0].width, 100.0);
        assert_eq!(scene.sprites[0].vx, 20.0);
        assert!(!scene.sprites[0].visible);
        assert_eq!(scene.sprites[0].asset.as_deref(), Some("Images/player.png"));
        let mut runtime = crate::runtime::Runtime::start(&scene).unwrap();
        runtime
            .advance(1.0 / 60.0, crate::runtime::Input::default())
            .unwrap();
        assert!(runtime.scene.sprites[0].x > 11.0);
    }

    #[test]
    fn rejects_duplicate_ids_and_invalid_transforms() {
        let mut level = LevelDocument {
            version: 1,
            name: "Level".into(),
            instances: vec![instance(), instance()],
        };
        assert!(Document::Level(level.clone()).validate().is_err());
        level.instances.pop();
        level.instances[0].scale_x = 0.0;
        assert!(Document::Level(level).validate().is_err());
    }

    #[test]
    fn persistence_uses_existing_sandbox_and_atomic_writes() {
        let root = tempfile::tempdir().unwrap();
        let path =
            project_files::create_file(root.path(), root.path(), "Player.altobject").unwrap();
        let document = Document::Object(ObjectDocument::new("Player"));
        save(root.path(), &path, &document).unwrap();
        assert_eq!(
            load(root.path(), &path).unwrap().to_toml().unwrap(),
            document.to_toml().unwrap()
        );
        let outside = tempfile::tempdir().unwrap();
        let escaped = outside.path().join("Player.altobject");
        std::fs::write(&escaped, "").unwrap();
        assert!(save(root.path(), &escaped, &document).is_err());
        assert!(load(root.path(), &escaped).is_err());
    }

    #[test]
    fn finds_and_rewrites_file_and_folder_references() {
        let mut object = ObjectDocument::new("Player");
        object.appearance.image = Some("Art/Hero/player.png".into());
        object.behavior.script = Some("Scripts/player.lua".into());
        let mut document = Document::Object(object);
        assert!(document.references_under("art"));
        assert!(document.references_under("Art/Hero/"));
        assert!(document.references_under("Scripts/player.lua"));
        assert!(!document.references_under("Ar"));
        assert!(!document.references_under("Scripts/player"));
        assert!(document.rewrite_references("Art", "Images"));
        assert!(document.rewrite_references("Scripts/player.lua", "Scripts/hero.lua"));
        assert!(!document.rewrite_references("Missing", "Other"));
        let Document::Object(object) = &document else {
            panic!("object")
        };
        assert_eq!(
            object.appearance.image.as_deref(),
            Some("Images/Hero/player.png")
        );
        assert_eq!(object.behavior.script.as_deref(), Some("Scripts/hero.lua"));

        let mut level = Document::Level(LevelDocument {
            version: 1,
            name: "Level".into(),
            instances: vec![instance()],
        });
        assert!(level.rewrite_references("Player.altobject", "Objects/Player.altobject"));
        assert!(level.references_under("Objects"));
        let mut game = Document::Game(GameDocument {
            version: 1,
            name: "Game".into(),
            starting_level: Some("Levels/A.altscene".into()),
        });
        assert!(game.rewrite_references("Levels", "Stages"));
        assert_eq!(game.references(), vec!["Stages/A.altscene".to_owned()]);
    }
}
