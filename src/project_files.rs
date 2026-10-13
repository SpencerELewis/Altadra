use std::fs;
use std::path::{Path, PathBuf};

const MAX_TEXT_FILE_SIZE: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryAction {
    NewFile,
    NewFolder,
    Rename,
}

pub fn validate_entry_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.trim() != name
        || name.len() > 120
        || name == "."
        || name == ".."
        || name.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
        })
    {
        return Err("Names must be 1–120 characters and cannot contain path separators or Windows-invalid characters.".into());
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
    {
        return Err("This name is reserved by Windows.".into());
    }
    Ok(())
}

pub fn create_file(root: &Path, parent: &Path, name: &str) -> Result<PathBuf, String> {
    validate_entry_name(name)?;
    let parent = checked_path(root, parent, true)?;
    if !parent.is_dir() {
        return Err("The selected parent is not a folder.".into());
    }
    if parent == root.canonicalize().map_err(|error| error.to_string())?
        && name.eq_ignore_ascii_case("scene.json")
    {
        return Err("scene.json is created and managed by the scene editor.".into());
    }
    let destination = parent.join(name);
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
    {
        Ok(_) => Ok(destination),
        Err(error) => Err(format!(
            "Could not create '{}': {error}",
            destination.display()
        )),
    }
}

pub fn create_folder(root: &Path, parent: &Path, name: &str) -> Result<PathBuf, String> {
    validate_entry_name(name)?;
    let parent = checked_path(root, parent, true)?;
    if !parent.is_dir() {
        return Err("The selected parent is not a folder.".into());
    }
    if parent == root.canonicalize().map_err(|error| error.to_string())?
        && name.eq_ignore_ascii_case("scene.json")
    {
        return Err("scene.json is managed by the scene editor.".into());
    }
    let destination = parent.join(name);
    fs::create_dir(&destination)
        .map_err(|error| format!("Could not create '{}': {error}", destination.display()))?;
    Ok(destination)
}

pub fn rename_entry(root: &Path, entry: &Path, name: &str) -> Result<PathBuf, String> {
    validate_entry_name(name)?;
    let entry = checked_path(root, entry, false)?;
    ensure_managed_scene_is_unchanged(root, &entry)?;
    let parent = entry
        .parent()
        .ok_or_else(|| "The project root cannot be renamed.".to_owned())?;
    if parent == root.canonicalize().map_err(|error| error.to_string())?
        && name.eq_ignore_ascii_case("scene.json")
    {
        return Err("scene.json is managed by the scene editor.".into());
    }
    let destination = parent.join(name);
    if destination.exists() {
        return Err(format!("'{}' already exists.", destination.display()));
    }
    fs::rename(&entry, &destination)
        .map_err(|error| format!("Could not rename '{}': {error}", entry.display()))?;
    Ok(destination)
}

pub fn delete_entry(root: &Path, entry: &Path) -> Result<(), String> {
    let entry = checked_path(root, entry, false)?;
    ensure_managed_scene_is_unchanged(root, &entry)?;
    let metadata = fs::symlink_metadata(&entry).map_err(|error| error.to_string())?;
    if metadata.is_dir() {
        fs::remove_dir_all(&entry)
    } else {
        fs::remove_file(&entry)
    }
    .map_err(|error| format!("Could not remove '{}': {error}", entry.display()))
}

pub fn read_text_file(root: &Path, file: &Path) -> Result<String, String> {
    let file = checked_path(root, file, false)?;
    ensure_managed_scene_is_unchanged(root, &file)?;
    let metadata = fs::metadata(&file).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Select a file to edit.".into());
    }
    if metadata.len() > MAX_TEXT_FILE_SIZE {
        return Err("Text files larger than 1 MB cannot be opened in the editor.".into());
    }
    let bytes = fs::read(&file).map_err(|error| format!("Could not read file: {error}"))?;
    String::from_utf8(bytes).map_err(|_| "This file is not UTF-8 text and cannot be edited.".into())
}

pub fn write_text_file(root: &Path, file: &Path, contents: &str) -> Result<(), String> {
    let file = checked_path(root, file, false)?;
    ensure_managed_scene_is_unchanged(root, &file)?;
    if contents.len() as u64 > MAX_TEXT_FILE_SIZE {
        return Err("Text files are limited to 1 MB.".into());
    }
    let metadata = fs::metadata(&file).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Select a file to edit.".into());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(
        file.parent()
            .ok_or_else(|| "The selected file has no parent folder.".to_owned())?,
    )
    .map_err(|error| format!("Could not prepare file save: {error}"))?;
    use std::io::Write;
    temporary
        .write_all(contents.as_bytes())
        .map_err(|error| format!("Could not write file: {error}"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("Could not flush file: {error}"))?;
    temporary
        .persist(&file)
        .map_err(|error| format!("Could not replace file: {}", error.error))?;
    Ok(())
}

pub fn stage_asset_file(root: &Path, asset_name: &str, contents: &[u8]) -> Result<(), String> {
    crate::project::validate_asset_name(asset_name)?;
    let parent = root.join("assets");
    if parent.exists() {
        checked_path(root, &parent, true)?;
    } else {
        create_folder(root, root, "assets")?;
    }
    let file_name = Path::new(asset_name)
        .file_name()
        .ok_or_else(|| "Invalid asset path.".to_owned())?
        .to_string_lossy();
    let file = parent.join(file_name.as_ref());
    if file.exists() {
        let file = checked_path(root, &file, false)?;
        let existing = fs::read(&file).map_err(|error| format!("Could not read asset: {error}"))?;
        if existing != contents {
            return Err(format!(
                "Asset '{}' already exists with different data.",
                asset_name
            ));
        }
        return Ok(());
    }
    let file = create_file(root, &parent, file_name.as_ref())?;
    fs::write(&file, contents)
        .map_err(|error| format!("Could not stage asset '{}': {error}", asset_name))
}

pub fn copy_workspace_files(workspace: &Path, destination: &Path) -> Result<(), String> {
    let workspace = workspace
        .canonicalize()
        .map_err(|error| format!("Could not resolve temporary project files: {error}"))?;
    let destination = destination
        .canonicalize()
        .map_err(|error| format!("Could not resolve project folder: {error}"))?;
    if destination == workspace {
        return Ok(());
    }
    if destination.starts_with(&workspace) || workspace.starts_with(&destination) {
        return Err("Project folders cannot be copied into one another.".into());
    }
    copy_directory_contents(&workspace, &destination, &workspace)
}

fn copy_directory_contents(
    source: &Path,
    destination: &Path,
    workspace: &Path,
) -> Result<(), String> {
    for item in fs::read_dir(source)
        .map_err(|error| format!("Could not read temporary project files: {error}"))?
    {
        let item =
            item.map_err(|error| format!("Could not inspect temporary project file: {error}"))?;
        let source_path = item.path();
        let file_type = item
            .file_type()
            .map_err(|error| format!("Could not inspect '{}': {error}", source_path.display()))?;
        if file_type.is_symlink() {
            return Err(format!(
                "Cannot save symbolic link '{}'.",
                source_path.display()
            ));
        }
        let relative = source_path
            .strip_prefix(workspace)
            .map_err(|_| "Temporary project files are outside their workspace.".to_owned())?;
        if relative
            .to_string_lossy()
            .eq_ignore_ascii_case("scene.json")
        {
            continue;
        }
        let target_path = destination.join(relative);
        if file_type.is_dir() {
            fs::create_dir_all(&target_path).map_err(|error| {
                format!("Could not create '{}': {error}", target_path.display())
            })?;
            copy_directory_contents(&source_path, destination, workspace)?;
        } else if file_type.is_file() {
            if target_path.exists() {
                if fs::read(&source_path).map_err(|error| error.to_string())?
                    == fs::read(&target_path).map_err(|error| error.to_string())?
                {
                    continue;
                }
                return Err(format!(
                    "Cannot save project file because '{}' already exists.",
                    target_path.display()
                ));
            }
            fs::copy(&source_path, &target_path).map_err(|error| {
                format!(
                    "Could not copy '{}' to '{}': {error}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
        } else {
            return Err(format!(
                "Unsupported project file type: '{}'.",
                source_path.display()
            ));
        }
    }
    Ok(())
}

fn ensure_managed_scene_is_unchanged(root: &Path, entry: &Path) -> Result<(), String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("Could not resolve project folder: {error}"))?;
    if entry == root.join("scene.json") {
        return Err("scene.json is managed by the scene editor and cannot be changed here.".into());
    }
    Ok(())
}

fn checked_path(root: &Path, target: &Path, allow_root: bool) -> Result<PathBuf, String> {
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("Could not resolve project folder: {error}"))?;
    let target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        canonical_root.join(target)
    };
    let (base, relative) = if let Ok(relative) = target.strip_prefix(root) {
        (root, relative)
    } else if let Ok(relative) = target.strip_prefix(&canonical_root) {
        (canonical_root.as_path(), relative)
    } else {
        return Err("File operations must stay inside the project folder.".into());
    };
    let mut current = base.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("File operations cannot follow symbolic links.".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(format!("Could not inspect path: {error}")),
        }
    }
    let target = target
        .canonicalize()
        .map_err(|error| format!("Could not resolve project path: {error}"))?;
    if !target.starts_with(&canonical_root) || (!allow_root && target == canonical_root) {
        return Err("File operations must stay inside the project folder.".into());
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_edits_renames_and_removes_project_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let folder = create_folder(root, root, "scripts").unwrap();
        let file = create_file(root, &folder, "player.lua").unwrap();

        write_text_file(root, &file, "return function() end").unwrap();
        assert_eq!(
            read_text_file(root, &file).unwrap(),
            "return function() end"
        );
        let renamed = rename_entry(root, &file, "hero.lua").unwrap();
        assert_eq!(
            read_text_file(root, &renamed).unwrap(),
            "return function() end"
        );
        delete_entry(root, &folder).unwrap();
        assert!(!folder.exists());
    }

    #[test]
    fn rejects_unsafe_names_paths_and_managed_scene_file() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        assert!(validate_entry_name("../outside").is_err());
        assert!(create_file(root, root, "bad/name.txt").is_err());
        assert!(create_file(root, root, "scene.json").is_err());
        assert!(delete_entry(root, root).is_err());

        let outside = root.join("..").join("outside");
        assert!(create_file(root, &outside, "escape.txt").is_err());

        fs::write(root.join("scene.json"), "{}").unwrap();
        assert!(read_text_file(root, &root.join("scene.json")).is_err());
        assert!(rename_entry(root, &root.join("scene.json"), "backup.json").is_err());
        assert!(delete_entry(root, &root.join("scene.json")).is_err());
    }

    #[test]
    fn rejects_binary_and_oversized_text_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let binary = create_file(root, root, "image.bin").unwrap();
        fs::write(&binary, [0xFF, 0xFE]).unwrap();
        assert!(read_text_file(root, &binary).unwrap_err().contains("UTF-8"));

        let large = create_file(root, root, "large.txt").unwrap();
        fs::write(&large, vec![b'a'; MAX_TEXT_FILE_SIZE as usize + 1]).unwrap();
        assert!(read_text_file(root, &large).unwrap_err().contains("1 MB"));
    }

    #[test]
    fn copies_unsaved_workspace_files_into_project_folder() {
        let workspace = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let folder = create_folder(workspace.path(), workspace.path(), "scripts").unwrap();
        let file = create_file(workspace.path(), &folder, "main.lua").unwrap();
        write_text_file(workspace.path(), &file, "return function() end").unwrap();
        stage_asset_file(workspace.path(), "assets/0123abcd.png", b"asset bytes").unwrap();

        let mut project = crate::project::Project {
            scene: crate::scene::Scene {
                version: 1,
                sprites: Vec::new(),
            },
            path: None,
            assets: Default::default(),
        };
        project.save(destination.path()).unwrap();
        copy_workspace_files(workspace.path(), destination.path()).unwrap();

        assert!(destination.path().join("scene.json").is_file());
        assert_eq!(
            fs::read_to_string(destination.path().join("scripts").join("main.lua")).unwrap(),
            "return function() end"
        );
        assert_eq!(
            fs::read(destination.path().join("assets").join("0123abcd.png")).unwrap(),
            b"asset bytes"
        );
    }

    #[test]
    fn copies_user_files_between_saved_projects_without_replacing_scene() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let mut project = crate::project::Project {
            scene: crate::scene::Scene {
                version: 1,
                sprites: Vec::new(),
            },
            path: None,
            assets: Default::default(),
        };
        project.save(source.path()).unwrap();
        fs::write(source.path().join("notes.txt"), "project notes").unwrap();
        project.save(destination.path()).unwrap();

        copy_workspace_files(source.path(), destination.path()).unwrap();

        assert!(destination.path().join("scene.json").is_file());
        assert_eq!(
            fs::read_to_string(destination.path().join("notes.txt")).unwrap(),
            "project notes"
        );
    }
}
