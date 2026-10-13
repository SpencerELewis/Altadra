use crate::scene::Scene;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectFile {
    scene: Scene,
}

#[derive(Default)]
pub struct Project {
    pub scene: Scene,
    pub path: Option<PathBuf>,
    pub assets: HashMap<String, Vec<u8>>,
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

impl Project {
    pub fn import_png(&mut self, bytes: &[u8]) -> Result<String, String> {
        validate_png_header(bytes)?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("PNG is limited to 64 MB.".into());
        }
        let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .map_err(|e| format!("Could not decode PNG: {e}"))?;
        if image.width() > 4096 || image.height() > 4096 {
            return Err("PNG dimensions are limited to 4096 × 4096.".into());
        }
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

    pub fn load(directory: &Path) -> Result<Self, String> {
        reject_symlink(directory)?;
        let root = directory
            .canonicalize()
            .map_err(|e| format!("Could not open project folder: {e}"))?;
        reject_symlink(&root)?;
        let scene_path = root.join("scene.json");
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
        let mut project = Self {
            scene: file.scene,
            path: Some(root.clone()),
            assets: HashMap::new(),
        };
        for name in project
            .scene
            .sprites
            .iter()
            .filter_map(|s| s.asset.as_ref())
            .collect::<std::collections::HashSet<_>>()
        {
            validate_asset_name(name)?;
            let path = root.join(name);
            reject_symlink(&root.join("assets"))?;
            reject_symlink(&path)?;
            let metadata =
                fs::metadata(&path).map_err(|e| format!("Missing asset '{name}': {e}"))?;
            if metadata.len() > 64 * 1024 * 1024 {
                return Err(format!("Image '{name}' exceeds 64 MB."));
            }
            let bytes = fs::read(path).map_err(|e| e.to_string())?;
            validate_png_header(&bytes)?;
            image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
                .map_err(|e| format!("Corrupt image '{name}': {e}"))?;
            project.assets.insert(name.to_owned(), bytes);
        }
        Ok(project)
    }

    pub fn save(&mut self, directory: &Path) -> Result<(), String> {
        self.scene.validate()?;
        reject_symlink(directory)?;
        let root = if directory.exists() {
            directory
                .canonicalize()
                .map_err(|e| format!("Could not resolve project folder: {e}"))?
        } else {
            directory.to_path_buf()
        };
        if root.exists() {
            reject_symlink(&root)?;
        }
        fs::create_dir_all(&root).map_err(|e| format!("Could not create project folder: {e}"))?;
        let assets_dir = root.join("assets");
        if assets_dir.exists() {
            reject_symlink(&assets_dir)?;
        }
        for name in self
            .scene
            .sprites
            .iter()
            .filter_map(|s| s.asset.as_ref())
            .collect::<std::collections::HashSet<_>>()
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
        let destination = root.join("scene.json");
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

fn validate_png_header(bytes: &[u8]) -> Result<(), String> {
    let signature = [137, 80, 78, 71, 13, 10, 26, 10];
    if bytes.len() > 64 * 1024 * 1024 {
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
    fn portable_scene_save_load_and_save_as() {
        let temporary = tempfile::tempdir().unwrap();
        let mut project = Project::default();
        project.save(&temporary.path().join("original")).unwrap();
        let mut loaded = Project::load(&temporary.path().join("original")).unwrap();
        loaded.save(&temporary.path().join("copy")).unwrap();
        let copied = Project::load(&temporary.path().join("copy")).unwrap();
        assert_eq!(project.scene.sprites.len(), copied.scene.sprites.len());
    }

    #[test]
    fn saving_without_images_does_not_create_assets_folder() {
        let temporary = tempfile::tempdir().unwrap();
        let mut project = Project::default();
        project.save(&temporary.path().join("game")).unwrap();
        assert!(temporary.path().join("game").join("scene.json").exists());
        assert!(!temporary.path().join("game").join("assets").exists());
        assert!(Project::load(&temporary.path().join("game")).is_ok());
    }

    #[test]
    fn png_assets_round_trip_between_project_folders() {
        use image::ImageEncoder;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[255, 128, 64, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let mut project = Project::default();
        let asset = project.import_png(&png).unwrap();
        project.scene.sprites[0].asset = Some(asset.clone());
        project.save(&temporary.path().join("original")).unwrap();
        let mut loaded = Project::load(&temporary.path().join("original")).unwrap();
        loaded.save(&temporary.path().join("copy")).unwrap();
        std::fs::rename(
            temporary.path().join("original"),
            temporary.path().join("moved"),
        )
        .unwrap();
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
        assert!(
            validate_png_header(&png)
                .unwrap_err()
                .contains("dimensions")
        );
    }
}
