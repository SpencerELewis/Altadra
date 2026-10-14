use crate::documents::{
    self, Appearance, Behavior, Document, GameDocument, Instance, LevelDocument, Motion,
    ObjectDocument, Overrides,
};
use crate::project_files;
use crate::scene::Scene;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_MANIFEST: &str = "game.altgame";
pub const LEGACY_SCENE: &str = "scene.json";
pub const MAX_PNG_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectFile {
    scene: Scene,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProjectFormat {
    /// `.altgame` manifest plus TOML documents (all new projects).
    #[default]
    Manifest,
    /// Legacy single `scene.json` project; saved in place as `scene.json`.
    Legacy,
}

pub struct Project {
    /// Manifest projects: the resolved starting level. Legacy projects: scene.json.
    pub scene: Scene,
    pub path: Option<PathBuf>,
    /// Decoded-and-validated PNG bytes keyed by project-relative `/` reference.
    pub assets: HashMap<String, Vec<u8>>,
    pub manifest: GameDocument,
    /// Manifest file name relative to the project root (default `game.altgame`).
    pub manifest_path: PathBuf,
    pub format: ProjectFormat,
    /// Non-fatal problem found while loading (e.g. a broken starting level).
    pub load_warning: Option<String>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            scene: Scene {
                version: 1,
                sprites: Vec::new(),
            },
            path: None,
            assets: HashMap::new(),
            manifest: GameDocument {
                version: 1,
                name: "Untitled Game".into(),
                starting_level: None,
            },
            manifest_path: PathBuf::from(DEFAULT_MANIFEST),
            format: ProjectFormat::Manifest,
            load_warning: None,
        }
    }
}

pub fn validate_asset_name(name: &str) -> Result<(), String> {
    let Some(file) = name.strip_prefix("assets/") else {
        return Err("Assets must be relative paths under assets/.".into());
    };
    if file.is_empty()
        || !file.to_ascii_lowercase().ends_with(".png")
        || file.contains("..")
        || !file
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
        || file.starts_with('/')
        || file.contains('/')
    {
        return Err(format!("Unsafe PNG asset path '{name}'."));
    }
    Ok(())
}

pub fn validate_project_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 80 {
        return Err("Project names must contain 1 to 80 characters.".into());
    }
    if name.ends_with([' ', '.'])
        || name.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
        })
    {
        return Err(
            "Project names cannot contain path separators or Windows-invalid characters.".into(),
        );
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
    {
        return Err("This name is reserved by Windows; choose another project name.".into());
    }
    Ok(())
}

/// Validates a PNG fully: size, header dimensions (checked before decoding), and decode.
pub fn validate_png(bytes: &[u8]) -> Result<(), String> {
    validate_png_header(bytes)?;
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|e| format!("Could not decode PNG: {e}"))?;
    if image.width() > 4096 || image.height() > 4096 {
        return Err("PNG dimensions are limited to 4096 × 4096.".into());
    }

    Ok(())
}

pub fn decode_image(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    if bytes.len() as u64 > MAX_PNG_BYTES {
        return Err("Images are limited to 64 MB.".into());
    }
    let format = image::guess_format(bytes).map_err(|error| format!("Invalid image: {error}"))?;
    if !matches!(
        format,
        image::ImageFormat::Png
            | image::ImageFormat::Jpeg
            | image::ImageFormat::WebP
            | image::ImageFormat::Bmp
            | image::ImageFormat::Gif
    ) {
        return Err("Supported images are PNG, JPEG, WebP, BMP, and GIF.".into());
    }
    let reader = || image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let (width, height) = reader()
        .into_dimensions()
        .map_err(|error| format!("Could not read image dimensions: {error}"))?;
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err("Image dimensions are limited to 1-4096 pixels.".into());
    }
    let mut reader = reader();
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|error| format!("Could not decode image: {error}"))
}

impl Project {
    pub fn import_image(&mut self, bytes: &[u8], name: &str) -> Result<String, String> {
        documents::validate_image_reference(name)?;
        decode_image(bytes)?;
        self.assets.insert(name.to_owned(), bytes.to_vec());
        Ok(name.to_owned())
    }
    #[cfg(test)]
    pub fn import_png(&mut self, bytes: &[u8]) -> Result<String, String> {
        validate_png(bytes)?;
        let digest = Sha256::digest(bytes);
        let name = format!(
            "assets/{}.png",
            digest
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        self.assets
            .entry(name.clone())
            .or_insert_with(|| bytes.to_vec());
        Ok(name)
    }

    /// Returns the root-relative manifest file name, if the folder has one.
    /// A project has at most one root `*.altgame`; several are an error.
    pub fn find_manifest(root: &Path) -> Result<Option<PathBuf>, String> {
        reject_symlink(root)?;
        let mut found = Vec::new();
        for entry in
            fs::read_dir(root).map_err(|e| format!("Could not read project folder: {e}"))?
        {
            let entry = entry.map_err(|e| format!("Could not inspect project folder: {e}"))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.to_ascii_lowercase().ends_with(".altgame") {
                continue;
            }
            let file_type = entry.file_type().map_err(|e| e.to_string())?;
            if file_type.is_symlink() {
                return Err(format!(
                    "Project manifest '{name}' cannot be a symbolic link."
                ));
            }
            if file_type.is_file() {
                found.push(name);
            }
        }
        match found.len() {
            0 => Ok(None),
            1 => Ok(found.pop().map(PathBuf::from)),
            _ => Err("The project folder contains multiple .altgame manifests.".into()),
        }
    }

    /// True for an old project that has `scene.json` but no `.altgame` manifest.
    pub fn is_legacy(root: &Path) -> bool {
        matches!(Self::find_manifest(root), Ok(None)) && root.join(LEGACY_SCENE).is_file()
    }

    /// True if the folder looks like an Altadra project (manifest or legacy).
    pub fn is_project_folder(root: &Path) -> bool {
        matches!(Self::find_manifest(root), Ok(Some(_))) || root.join(LEGACY_SCENE).is_file()
    }

    /// Opens a project without modifying anything on disk. Prefers an `.altgame`
    /// manifest and falls back to legacy `scene.json`.
    pub fn load(directory: &Path) -> Result<Self, String> {
        reject_symlink(directory)?;
        let root = directory
            .canonicalize()
            .map_err(|e| format!("Could not open project folder: {e}"))?;
        reject_symlink(&root)?;
        match Self::find_manifest(&root)? {
            Some(manifest_path) => Self::load_manifest(root, manifest_path),
            None if root.join(LEGACY_SCENE).exists() => Self::load_legacy(root),
            None => Err(format!(
                "'{}' has no .altgame manifest or scene.json.",
                root.display()
            )),
        }
    }

    fn load_manifest(root: PathBuf, manifest_path: PathBuf) -> Result<Self, String> {
        let Document::Game(manifest) = documents::load(&root, &root.join(&manifest_path))? else {
            return Err("The project manifest is not a game document.".into());
        };
        let mut project = Self {
            path: Some(root.clone()),
            manifest,
            manifest_path,
            ..Self::default()
        };
        if let Some(level) = project.manifest.starting_level.clone() {
            match Self::resolve_level_from_disk(&root, &level) {
                Ok((scene, assets)) => {
                    project.scene = scene;
                    project.assets = assets;
                }
                Err(error) => {
                    project.load_warning = Some(format!(
                        "Starting level '{level}' could not be loaded: {error}"
                    ))
                }
            }
        }
        Ok(project)
    }

    fn load_legacy(root: PathBuf) -> Result<Self, String> {
        let scene_path = root.join(LEGACY_SCENE);
        reject_symlink(&scene_path)?;
        let metadata =
            fs::metadata(&scene_path).map_err(|e| format!("Could not read scene.json: {e}"))?;
        if metadata.len() > 16 * 1024 * 1024 {
            return Err("scene.json is limited to 16 MB.".into());
        }
        let file: ProjectFile =
            serde_json::from_slice(&fs::read(scene_path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("Invalid scene.json: {e}"))?;
        file.scene.validate()?;
        let mut name: String = root
            .file_name()
            .map(|name| name.to_string_lossy().trim().to_owned())
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect();
        if name.is_empty() {
            name = "Untitled Game".into();
        }
        let mut project = Self {
            scene: file.scene,
            path: Some(root.clone()),
            format: ProjectFormat::Legacy,
            ..Self::default()
        };
        project.manifest.name = name;
        for name in project
            .scene
            .sprites
            .iter()
            .filter_map(|s| s.asset.as_ref())
            .collect::<HashSet<_>>()
        {
            validate_asset_name(name)?;
            let path = root.join(name);
            reject_symlink(&root.join("assets"))?;
            reject_symlink(&path)?;
            let metadata =
                fs::metadata(&path).map_err(|e| format!("Missing asset '{name}': {e}"))?;
            if metadata.len() > MAX_PNG_BYTES {
                return Err(format!("Image '{name}' exceeds 64 MB."));
            }
            let bytes = fs::read(path).map_err(|e| e.to_string())?;
            validate_png(&bytes).map_err(|e| format!("Corrupt image '{name}': {e}"))?;
            project.assets.insert(name.to_owned(), bytes);
        }
        Ok(project)
    }

    /// Resolves a level into a runtime scene plus the PNGs it references.
    /// Documents and scripts come from the given readers (so unsaved editor
    /// snapshots can be used); images are read from disk via the sandbox.
    pub fn resolve_scene(
        root: &Path,
        level: &LevelDocument,
        read_document: impl FnMut(&Path) -> Result<Document, String>,
        read_script: impl FnMut(&Path) -> Result<String, String>,
    ) -> Result<(Scene, HashMap<String, Vec<u8>>), String> {
        let scene = documents::resolve_level(root, level, read_document, read_script)?;
        let mut assets = HashMap::new();
        for sprite in &scene.sprites {
            if let Some(image) = &sprite.asset
                && !assets.contains_key(image)
            {
                let bytes =
                    project_files::read_image(root, &documents::reference_path(root, image))
                        .map_err(|e| format!("Image '{image}': {e}"))?;
                assets.insert(image.clone(), bytes);
            }
        }
        Ok((scene, assets))
    }

    /// `resolve_scene` for a level reference, reading everything from disk.
    pub fn resolve_level_from_disk(
        root: &Path,
        level: &str,
    ) -> Result<(Scene, HashMap<String, Vec<u8>>), String> {
        let Document::Level(level) =
            documents::load(root, &documents::reference_path(root, level))?
        else {
            return Err(format!("'{level}' is not a level document."));
        };
        Self::resolve_scene(
            root,
            &level,
            |path| documents::load(root, path),
            |path| project_files::read_text_file(root, path),
        )
    }

    /// Saves to `directory` (created if needed). Manifest projects write the
    /// manifest and in-memory PNGs only (never scene.json; level/object/script
    /// documents are saved by their editors). Legacy projects write scene.json.
    pub fn save(&mut self, directory: &Path) -> Result<(), String> {
        match self.format {
            ProjectFormat::Manifest => self.save_manifest_project(directory),
            ProjectFormat::Legacy => self.save_legacy(directory),
        }
    }

    fn prepare_root(directory: &Path) -> Result<PathBuf, String> {
        reject_symlink(directory)?;
        fs::create_dir_all(directory)
            .map_err(|e| format!("Could not create project folder: {e}"))?;
        let root = directory
            .canonicalize()
            .map_err(|e| format!("Could not resolve project folder: {e}"))?;
        reject_symlink(&root)?;
        Ok(root)
    }

    fn validate_manifest_path(&self) -> Result<String, String> {
        let name = self
            .manifest_path
            .to_str()
            .filter(|name| self.manifest_path.components().count() == 1 && !name.is_empty())
            .ok_or_else(|| "The project manifest must be a file in the project root.".to_owned())?;
        project_files::validate_entry_name(name)?;
        if !name.to_ascii_lowercase().ends_with(".altgame") {
            return Err("The project manifest must use the .altgame extension.".into());
        }
        Ok(name.to_owned())
    }

    fn save_manifest_project(&mut self, directory: &Path) -> Result<(), String> {
        let manifest_name = self.validate_manifest_path()?;
        let contents = Document::Game(self.manifest.clone()).to_toml()?;
        let mut assets: Vec<_> = self.assets.iter().collect();
        assets.sort_by(|left, right| left.0.cmp(right.0));
        for (name, bytes) in &assets {
            documents::validate_image_reference(name)?;
            decode_image(bytes).map_err(|e| format!("Image '{name}': {e}"))?;
        }
        let root = Self::prepare_root(directory)?;
        if let Some(existing) = Self::find_manifest(&root)?
            && !existing
                .to_string_lossy()
                .eq_ignore_ascii_case(&manifest_name)
        {
            return Err(format!(
                "The destination already contains another project manifest ('{}').",
                existing.display()
            ));
        }
        for (name, bytes) in assets {
            project_files::stage_file(&root, name, bytes)?;
        }
        let manifest = root.join(&manifest_name);
        if manifest.exists() {
            project_files::write_text_file(&root, &manifest, &contents)?;
        } else {
            project_files::create_text_file(&root, &root, &manifest_name, &contents)?;
        }
        self.path = Some(root);
        Ok(())
    }

    fn save_legacy(&mut self, directory: &Path) -> Result<(), String> {
        self.scene.validate()?;
        let root = Self::prepare_root(directory)?;
        let assets_dir = root.join("assets");
        if assets_dir.exists() {
            reject_symlink(&assets_dir)?;
        }
        for name in self
            .scene
            .sprites
            .iter()
            .filter_map(|s| s.asset.as_ref())
            .collect::<HashSet<_>>()
        {
            validate_asset_name(name)?;
            let bytes = self
                .assets
                .get(name)
                .ok_or_else(|| format!("Missing imported image '{name}'."))?;
            let path = root.join(name);
            if path.exists() {
                reject_symlink(&path)?;
                let existing = fs::read(&path).map_err(|e| e.to_string())?;
                if existing != *bytes {
                    return Err(format!(
                        "Destination image '{name}' has different contents."
                    ));
                }
            } else {
                fs::create_dir_all(&assets_dir).map_err(|e| e.to_string())?;
                fs::write(path, bytes)
                    .map_err(|e| format!("Could not save image '{name}': {e}"))?;
            }
        }
        let data = serde_json::to_vec_pretty(&ProjectFile {
            scene: self.scene.clone(),
        })
        .map_err(|e| e.to_string())?;
        if data.len() > 16 * 1024 * 1024 {
            return Err("Serialized scene exceeds the 16 MB project limit.".into());
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&root).map_err(|e| e.to_string())?;
        use std::io::Write;
        temporary.write_all(&data).map_err(|e| e.to_string())?;
        temporary.as_file().sync_all().map_err(|e| e.to_string())?;
        let destination = root.join(LEGACY_SCENE);
        if destination.exists() {
            reject_symlink(&destination)?;
        }
        temporary
            .persist(&destination)
            .map_err(|e| format!("Could not replace scene.json: {}", e.error))?;
        self.path = Some(root);
        Ok(())
    }
}

/// Converts a legacy `scene.json` project in place into `game.altgame`,
/// `Levels/Main.altscene`, one `Objects/*.altobject` per sprite, and
/// `Scripts/*.lua` for inline scripts. Original files are left untouched;
/// on any failure only the files and folders created here are removed.
pub fn migrate_legacy(root: &Path) -> Result<Project, String> {
    if !Project::is_legacy(root) {
        return Err(
            "Only projects with scene.json and no .altgame manifest can be migrated.".into(),
        );
    }
    let legacy = Project::load(root)?;
    if legacy.format != ProjectFormat::Legacy {
        return Err("This project already has an .altgame manifest.".into());
    }
    let root = legacy
        .path
        .clone()
        .ok_or_else(|| "Legacy project has no folder.".to_owned())?;
    let plan = plan_migration(&legacy)?;
    preflight_migration(&root, &plan)?;
    let mut created = Created::default();
    let result = write_migration(&root, &plan, &mut created).and_then(|()| {
        let migrated = Project::load(&root)?;
        if let Some(warning) = &migrated.load_warning {
            return Err(warning.clone());
        }
        let expected = serde_json::to_value(&legacy.scene).map_err(|e| e.to_string())?;
        let actual = serde_json::to_value(&migrated.scene).map_err(|e| e.to_string())?;
        if expected != actual {
            return Err("Migrated documents do not reproduce the legacy scene.".into());
        }
        Ok(migrated)
    });
    result.map_err(|error| {
        created.rollback();
        format!("Migration failed; no changes were kept: {error}")
    })
}

const MIGRATION_FOLDERS: [&str; 3] = ["Levels", "Objects", "Scripts"];

struct MigrationPlan {
    /// (folder or "" for root, file name, contents)
    files: Vec<(&'static str, String, String)>,
}

fn plan_migration(legacy: &Project) -> Result<MigrationPlan, String> {
    let mut files = Vec::new();
    let mut used = HashSet::new();
    let mut instances = Vec::new();
    for sprite in &legacy.scene.sprites {
        let stem = unique_stem(&sprite.name, &mut used);
        let script = if sprite.script.is_empty() {
            None
        } else {
            files.push(("Scripts", format!("{stem}.lua"), sprite.script.clone()));
            Some(format!("Scripts/{stem}.lua"))
        };
        let object = ObjectDocument {
            version: 1,
            name: sprite.name.clone(),
            appearance: Appearance {
                image: sprite.asset.clone(),
                width: sprite.width,
                height: sprite.height,
                tint: sprite.tint,
                visible: sprite.visible,
            },
            behavior: Behavior { script },
            motion: Motion {
                vx: sprite.vx,
                vy: sprite.vy,
            },
        };
        files.push((
            "Objects",
            format!("{stem}.altobject"),
            Document::Object(object).to_toml()?,
        ));
        instances.push(Instance {
            id: sprite.id.clone(),
            object: format!("Objects/{stem}.altobject"),
            x: sprite.x,
            y: sprite.y,
            rotation: sprite.rotation,
            scale_x: sprite.scale_x,
            scale_y: sprite.scale_y,
            overrides: Overrides::default(),
        });
    }
    let level = LevelDocument {
        version: 1,
        name: "Main".into(),
        instances,
    };
    files.push((
        "Levels",
        "Main.altscene".into(),
        Document::Level(level).to_toml()?,
    ));
    let manifest = GameDocument {
        version: 1,
        name: legacy.manifest.name.clone(),
        starting_level: Some("Levels/Main.altscene".into()),
    };
    files.push((
        "",
        DEFAULT_MANIFEST.into(),
        Document::Game(manifest).to_toml()?,
    ));
    Ok(MigrationPlan { files })
}

fn unique_stem(name: &str, used: &mut HashSet<String>) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    let mut base = cleaned.trim().to_owned();
    if base.is_empty() {
        base = "Object".into();
    }
    if project_files::validate_entry_name(&format!("{base}.altobject")).is_err() {
        base = format!("Object {base}");
    }
    let mut candidate = base.clone();
    let mut index = 2;
    while !used.insert(candidate.to_lowercase()) {
        candidate = format!("{base} {index}");
        index += 1;
    }
    candidate
}

fn preflight_migration(root: &Path, plan: &MigrationPlan) -> Result<(), String> {
    let mut conflicts = Vec::new();
    for folder in MIGRATION_FOLDERS {
        let path = root.join(folder);
        if let Ok(metadata) = fs::symlink_metadata(&path)
            && !metadata.is_dir()
        {
            conflicts.push(folder.to_owned());
        }
    }
    for (folder, name, _) in &plan.files {
        let relative = if folder.is_empty() {
            name.clone()
        } else {
            format!("{folder}/{name}")
        };
        if fs::symlink_metadata(documents::reference_path(root, &relative)).is_ok() {
            conflicts.push(relative);
        }
    }
    if conflicts.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Migration would overwrite existing files: {}",
            conflicts.join(", ")
        ))
    }
}

#[derive(Default)]
struct Created {
    files: Vec<PathBuf>,
    folders: Vec<PathBuf>,
}

impl Created {
    fn rollback(&mut self) {
        for file in self.files.drain(..).rev() {
            let _ = fs::remove_file(file);
        }
        for folder in self.folders.drain(..).rev() {
            let _ = fs::remove_dir(folder);
        }
    }
}

fn write_migration(root: &Path, plan: &MigrationPlan, created: &mut Created) -> Result<(), String> {
    for folder in MIGRATION_FOLDERS {
        if !plan.files.iter().any(|(parent, _, _)| *parent == folder) {
            continue;
        }
        let path = root.join(folder);
        if !path.is_dir() {
            created
                .folders
                .push(project_files::create_folder(root, root, folder)?);
        }
    }
    for (folder, name, contents) in &plan.files {
        let parent = if folder.is_empty() {
            root.to_path_buf()
        } else {
            root.join(folder)
        };
        created.files.push(project_files::create_text_file(
            root, &parent, name, contents,
        )?);
    }
    Ok(())
}

fn validate_png_header(bytes: &[u8]) -> Result<(), String> {
    let signature = [137, 80, 78, 71, 13, 10, 26, 10];
    if bytes.len() as u64 > MAX_PNG_BYTES {
        return Err("PNG is limited to 64 MB.".into());
    }
    if bytes.len() < 33 || bytes[..8] != signature || &bytes[12..16] != b"IHDR" {
        return Err("Image is not a valid PNG.".into());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().expect("four-byte PNG width"));
    let height = u32::from_be_bytes(bytes[20..24].try_into().expect("four-byte PNG height"));
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err("PNG dimensions are limited to 1–4096 pixels.".into());
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors() {
        if ancestor.exists()
            && fs::symlink_metadata(ancestor)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
        {
            return Err(format!(
                "Symlinks are not allowed in project paths: {}",
                ancestor.display()
            ));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn additional_images_import_resolve_save_and_reopen() {
        for (format, extension) in [
            (image::ImageFormat::Jpeg, "jpg"),
            (image::ImageFormat::WebP, "webp"),
            (image::ImageFormat::Bmp, "bmp"),
            (image::ImageFormat::Gif, "gif"),
        ] {
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                2,
                2,
                image::Rgb([50, 100, 150]),
            ))
            .write_to(&mut bytes, format)
            .unwrap();
            let bytes = bytes.into_inner();
            let mut project = Project::default();
            let reference = project
                .import_image(&bytes, &format!("Art/Player.{extension}"))
                .unwrap();
            assert_eq!(project.import_image(&bytes, &reference).unwrap(), reference);
            assert_eq!(project.assets.len(), 1);
            assert!(reference.ends_with(extension));
            assert_eq!(decode_image(&bytes).unwrap().width(), 2);
            let root = tempfile::tempdir().unwrap();
            project.save(root.path()).unwrap();
            let mut object = crate::documents::ObjectDocument::new("Artwork");
            object.appearance.image = Some(reference.clone());
            let object_path =
                crate::project_files::create_file(root.path(), root.path(), "Art.altobject")
                    .unwrap();
            crate::documents::save(root.path(), &object_path, &Document::Object(object)).unwrap();
            let level = LevelDocument {
                version: 1,
                name: "Level".into(),
                instances: vec![crate::documents::Instance {
                    id: "art".into(),
                    object: "Art.altobject".into(),
                    x: 0.0,
                    y: 0.0,
                    rotation: 0.0,
                    scale_x: 1.0,
                    scale_y: 1.0,
                    overrides: Default::default(),
                }],
            };
            let level_path =
                crate::project_files::create_file(root.path(), root.path(), "Level.altscene")
                    .unwrap();
            crate::documents::save(root.path(), &level_path, &Document::Level(level)).unwrap();
            project.manifest.starting_level = Some("Level.altscene".into());
            project.save(root.path()).unwrap();
            let loaded = Project::load(root.path()).unwrap();
            assert!(loaded.load_warning.is_none(), "{:?}", loaded.load_warning);
            assert_eq!(
                loaded.scene.sprites[0].asset.as_deref(),
                Some(reference.as_str())
            );
            assert_eq!(loaded.assets[&reference], bytes);
        }
    }

    #[test]
    fn animated_gif_uses_only_first_frame() {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for color in [[255, 0, 0, 255], [0, 255, 0, 255]] {
                encoder
                    .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                        1,
                        1,
                        image::Rgba(color),
                    )))
                    .unwrap();
            }
        }
        assert_eq!(
            decode_image(&bytes).unwrap().to_rgba8().get_pixel(0, 0).0,
            [255, 0, 0, 255]
        );
        assert!(decode_image(b"invalid image").is_err());
        let mut huge = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::new(4097, 1))
            .write_to(
                &mut std::io::Cursor::new(&mut huge),
                image::ImageFormat::Bmp,
            )
            .unwrap();
        assert!(decode_image(&huge).unwrap_err().contains("dimensions"));
    }
    use crate::scene::Sprite;

    fn png(width: u32) -> Vec<u8> {
        use image::ImageEncoder;
        let mut png = Vec::new();
        let pixels = vec![200; (width * 4) as usize];
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&pixels, width, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        png
    }

    fn new_project(name: &str) -> Project {
        let mut project = Project::default();
        project.manifest.name = name.into();
        project
    }

    fn legacy_project() -> Project {
        Project {
            scene: Scene::default(),
            format: ProjectFormat::Legacy,
            ..Project::default()
        }
    }

    #[test]
    fn rejects_traversal_and_non_png_assets() {
        for name in [
            "../scene.json",
            "assets/../scene.json",
            "C:/image.png",
            "assets/foo.jpg",
            "assets/a/b.png",
        ] {
            assert!(validate_asset_name(name).is_err(), "accepted {name}");
        }
    }

    #[test]
    fn validates_new_project_folder_names() {
        for name in ["", ".", "..", "CON", "LPT1", "bad/name", "trailing."] {
            assert!(validate_project_name(name).is_err(), "accepted {name}");
        }
        for name in ["My Game", "Project_02", "árena"] {
            assert!(validate_project_name(name).is_ok(), "rejected {name}");
        }
    }

    #[test]
    fn default_project_creates_no_files_and_saves_manifest_only() {
        let temporary = tempfile::tempdir().unwrap();
        let mut project = new_project("My Game");
        assert!(project.path.is_none());
        assert!(project.scene.sprites.is_empty());
        let folder = temporary.path().join("game");
        project.save(&folder).unwrap();
        assert!(folder.join(DEFAULT_MANIFEST).is_file());
        assert!(!folder.join(LEGACY_SCENE).exists());
        assert!(!folder.join("assets").exists());
        assert!(Project::find_manifest(&folder).unwrap().is_some());
        assert!(!Project::is_legacy(&folder));
        assert!(Project::is_project_folder(&folder));
        let loaded = Project::load(&folder).unwrap();
        assert_eq!(loaded.format, ProjectFormat::Manifest);
        assert_eq!(loaded.manifest.name, "My Game");
        assert!(loaded.load_warning.is_none());
        // Re-saving replaces the manifest atomically.
        project.manifest.name = "Renamed".into();
        project.save(&folder).unwrap();
        assert_eq!(Project::load(&folder).unwrap().manifest.name, "Renamed");
    }

    #[test]
    fn manifest_detection_requires_exactly_one_manifest() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        assert_eq!(Project::find_manifest(root).unwrap(), None);
        fs::write(root.join("Other.altgame"), "").unwrap();
        assert_eq!(
            Project::find_manifest(root).unwrap(),
            Some(PathBuf::from("Other.altgame"))
        );
        fs::write(root.join("Second.altgame"), "").unwrap();
        assert!(Project::find_manifest(root).is_err());
        fs::remove_file(root.join("Second.altgame")).unwrap();
        fs::write(root.join(DEFAULT_MANIFEST), "").unwrap();
        assert!(Project::find_manifest(root).is_err());
        assert!(Project::load(root).is_err());
        fs::remove_file(root.join("Other.altgame")).unwrap();
        assert_eq!(
            Project::find_manifest(root).unwrap(),
            Some(PathBuf::from(DEFAULT_MANIFEST))
        );
        assert!(Project::load(tempfile::tempdir().unwrap().path()).is_err());
    }

    #[test]
    fn manifest_save_writes_imported_assets_and_load_resolves_starting_level() {
        let temporary = tempfile::tempdir().unwrap();
        let folder = temporary.path().join("game");
        let mut project = new_project("Game");
        let asset = project.import_png(&png(1)).unwrap();
        project.save(&folder).unwrap();
        let root = folder.canonicalize().unwrap();
        // Images may live anywhere under the project with arbitrary names.
        project_files::stage_file(&root, "Art/Hero Sprite.png", &png(2)).unwrap();
        let objects = project_files::create_folder(&root, &root, "Objects").unwrap();
        let mut object = ObjectDocument::new("Hero");
        object.appearance.image = Some("Art/Hero Sprite.png".into());
        object.behavior.script = Some("hero.lua".into());
        project_files::create_text_file(
            &root,
            &objects,
            "Hero.altobject",
            &Document::Object(object).to_toml().unwrap(),
        )
        .unwrap();
        project_files::create_text_file(&root, &root, "hero.lua", "return function() end").unwrap();
        let level = LevelDocument {
            version: 1,
            name: "Start".into(),
            instances: vec![Instance {
                id: "hero-1".into(),
                object: "Objects/Hero.altobject".into(),
                x: 5.0,
                y: 6.0,
                rotation: 0.0,
                scale_x: 1.0,
                scale_y: 1.0,
                overrides: Overrides {
                    tint: Some([1, 2, 3, 4]),
                    ..Overrides::default()
                },
            }],
        };
        project_files::create_text_file(
            &root,
            &root,
            "Start.altscene",
            &Document::Level(level).to_toml().unwrap(),
        )
        .unwrap();
        project.manifest.starting_level = Some("Start.altscene".into());
        project.save(&folder).unwrap();

        assert!(root.join(&asset).is_file());
        let loaded = Project::load(&folder).unwrap();
        assert!(loaded.load_warning.is_none(), "{:?}", loaded.load_warning);
        let sprite = &loaded.scene.sprites[0];
        assert_eq!(sprite.id, "hero-1");
        assert_eq!(sprite.tint, [1, 2, 3, 4]);
        assert_eq!(sprite.script, "return function() end");
        assert_eq!(loaded.assets["Art/Hero Sprite.png"], png(2));

        // In-memory snapshots take priority over disk through the readers.
        let mut snapshot = ObjectDocument::new("Snapshot");
        snapshot.appearance.width = 10.0;
        let Document::Level(level) = documents::load(&root, &root.join("Start.altscene")).unwrap()
        else {
            panic!("level")
        };
        let (scene, assets) = Project::resolve_scene(
            &root,
            &level,
            |_| Ok(Document::Object(snapshot.clone())),
            |_| Ok(String::new()),
        )
        .unwrap();
        assert_eq!(scene.sprites[0].width, 10.0);
        assert!(assets.is_empty());

        // A broken starting level is reported but the project still opens.
        fs::write(root.join("Start.altscene"), "broken").unwrap();
        let broken = Project::load(&folder).unwrap();
        assert!(broken.load_warning.is_some());
    }

    #[test]
    fn resolve_scene_rejects_corrupt_and_escaping_images() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        fs::write(root.join("bad.png"), b"not a png").unwrap();
        let mut object = ObjectDocument::new("Thing");
        object.appearance.image = Some("bad.png".into());
        let level = LevelDocument {
            version: 1,
            name: "L".into(),
            instances: vec![Instance {
                id: "a".into(),
                object: "Thing.altobject".into(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                scale_x: 1.0,
                scale_y: 1.0,
                overrides: Overrides::default(),
            }],
        };
        let resolve = |object: &ObjectDocument| {
            Project::resolve_scene(
                &root,
                &level,
                |_| Ok(Document::Object(object.clone())),
                |_| Ok(String::new()),
            )
        };
        assert!(resolve(&object).is_err());
        object.appearance.image = Some("../outside.png".into());
        assert!(resolve(&object).is_err());
    }

    #[test]
    fn manifest_save_refuses_conflicting_assets_and_manifests() {
        let temporary = tempfile::tempdir().unwrap();
        let folder = temporary.path().join("game");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("Other.altgame"), "").unwrap();
        let mut project = new_project("Game");
        assert!(project.save(&folder).is_err());
        fs::remove_file(folder.join("Other.altgame")).unwrap();
        let asset = project.import_png(&png(1)).unwrap();
        fs::create_dir(folder.join("assets")).unwrap();
        fs::write(folder.join(&asset), b"different").unwrap();
        assert!(project.save(&folder).is_err());
        project.manifest_path = PathBuf::from("../escape.altgame");
        assert!(project.save(&folder).is_err());
    }

    #[test]
    fn legacy_scene_save_load_and_save_as() {
        let temporary = tempfile::tempdir().unwrap();
        let mut project = legacy_project();
        project.save(&temporary.path().join("original")).unwrap();
        assert!(Project::is_legacy(&temporary.path().join("original")));
        let mut loaded = Project::load(&temporary.path().join("original")).unwrap();
        assert_eq!(loaded.format, ProjectFormat::Legacy);
        assert_eq!(loaded.manifest.name, "original");
        loaded.save(&temporary.path().join("copy")).unwrap();
        let copied = Project::load(&temporary.path().join("copy")).unwrap();
        assert_eq!(project.scene.sprites.len(), copied.scene.sprites.len());
        assert!(
            !temporary
                .path()
                .join("copy")
                .join(DEFAULT_MANIFEST)
                .exists()
        );
    }

    #[test]
    fn png_assets_round_trip_between_legacy_project_folders() {
        let png = png(1);
        let temporary = tempfile::tempdir().unwrap();
        let mut project = legacy_project();
        let asset = project.import_png(&png).unwrap();
        project.scene.sprites[0].asset = Some(asset.clone());
        project.save(&temporary.path().join("original")).unwrap();
        let mut loaded = Project::load(&temporary.path().join("original")).unwrap();
        loaded.save(&temporary.path().join("copy")).unwrap();
        let copy = Project::load(&temporary.path().join("copy")).unwrap();
        assert_eq!(copy.assets.get(&asset).unwrap(), &png);
    }

    #[test]
    fn oversized_png_dimensions_rejected_before_decode() {
        let mut png = vec![0; 33];
        png[..8].copy_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
        png[12..16].copy_from_slice(b"IHDR");
        png[16..20].copy_from_slice(&4097_u32.to_be_bytes());
        png[20..24].copy_from_slice(&1_u32.to_be_bytes());
        assert!(validate_png(&png).unwrap_err().contains("dimensions"));
        assert!(validate_png(b"garbage").is_err());
    }

    fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    files.push((path.clone(), Vec::new()));
                    stack.push(path);
                } else {
                    files.push((path.clone(), fs::read(&path).unwrap()));
                }
            }
        }
        files.sort();
        files
    }

    #[test]
    fn migration_preserves_every_sprite_property_and_originals() {
        let temporary = tempfile::tempdir().unwrap();
        let folder = temporary.path().join("Old Game");
        let mut project = legacy_project();
        let asset = project.import_png(&png(1)).unwrap();
        project.scene.sprites[0].asset = Some(asset.clone());
        project.scene.sprites[0].script =
            "return function(entity, input, dt) entity.x = entity.x + 1 end".into();
        project.scene.sprites[1].rotation = 0.25;
        project.scene.sprites[1].scale_x = 2.5;
        project.scene.sprites[1].visible = false;
        project.scene.sprites[2].name = "Platform".into();
        project.scene.sprites.push(Sprite {
            name: "Platform".into(),
            x: 0.1 + 0.2,
            ..Sprite::new("x")
        });
        project.save(&folder).unwrap();
        let legacy_json = fs::read(folder.join(LEGACY_SCENE)).unwrap();
        assert!(Project::is_legacy(&folder));
        // Loading never mutates.
        let before = snapshot(&folder);
        Project::load(&folder).unwrap();
        assert_eq!(before, snapshot(&folder));

        let migrated = migrate_legacy(&folder).unwrap();
        assert_eq!(migrated.format, ProjectFormat::Manifest);
        assert_eq!(migrated.manifest.name, "Old Game");
        assert_eq!(fs::read(folder.join(LEGACY_SCENE)).unwrap(), legacy_json);
        assert!(folder.join(&asset).is_file());
        assert!(folder.join("Scripts").is_dir());
        assert!(folder.join("Objects").join("Platform.altobject").is_file());
        assert!(
            folder
                .join("Objects")
                .join("Platform 2.altobject")
                .is_file()
        );
        assert_eq!(
            serde_json::to_value(&migrated.scene).unwrap(),
            serde_json::to_value(&project.scene).unwrap()
        );
        assert_eq!(migrated.assets[&asset], project.assets[&asset]);
        assert!(!Project::is_legacy(&folder));
        let reopened = Project::load(&folder).unwrap();
        assert_eq!(reopened.format, ProjectFormat::Manifest);
        assert!(migrate_legacy(&folder).is_err());
    }

    #[test]
    fn migration_preflights_conflicts_and_leaves_folder_unchanged() {
        let temporary = tempfile::tempdir().unwrap();
        let folder = temporary.path().join("game");
        let mut project = legacy_project();
        project.save(&folder).unwrap();
        fs::create_dir(folder.join("Levels")).unwrap();
        fs::write(folder.join("Levels").join("Main.altscene"), "mine").unwrap();
        let before = snapshot(&folder);
        let error = migrate_legacy(&folder).err().expect("conflict");
        assert!(error.contains("Levels/Main.altscene"), "{error}");
        assert_eq!(before, snapshot(&folder));

        fs::remove_dir_all(folder.join("Levels")).unwrap();
        fs::write(folder.join("Objects"), "a file").unwrap();
        let before = snapshot(&folder);
        assert!(migrate_legacy(&folder).is_err());
        assert_eq!(before, snapshot(&folder));
    }

    #[test]
    fn migration_rolls_back_created_files_on_failure() {
        let temporary = tempfile::tempdir().unwrap();
        let folder = temporary.path().join("game");
        let mut project = legacy_project();
        project.save(&folder).unwrap();
        let legacy = Project::load(&folder).unwrap();
        let root = legacy.path.clone().unwrap();
        let mut plan = plan_migration(&legacy).unwrap();
        // Force a failure part-way through by planning an oversized file last.
        plan.files
            .push(("Scripts", "huge.lua".into(), "x".repeat(2 * 1024 * 1024)));
        let before = snapshot(&root);
        preflight_migration(&root, &plan).unwrap();
        let mut created = Created::default();
        assert!(write_migration(&root, &plan, &mut created).is_err());
        assert!(!created.files.is_empty());
        created.rollback();
        assert_eq!(before, snapshot(&root));
    }

    #[test]
    fn unique_stems_are_safe_file_names() {
        let mut used = HashSet::new();
        assert_eq!(
            unique_stem("Player · Lua / WASD", &mut used),
            "Player _ Lua _ WASD"
        );
        assert_eq!(unique_stem("CON", &mut used), "Object CON");
        assert_eq!(unique_stem("  ", &mut used), "Object");
        assert_eq!(unique_stem("object", &mut used), "object 2");
    }
}
