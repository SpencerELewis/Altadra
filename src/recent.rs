use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_RECENT_PROJECTS: usize = 12;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentProject {
    pub name: String,
    pub path: PathBuf,
    pub opened_at: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecentProjects {
    pub projects: Vec<RecentProject>,
}

impl RecentProjects {
    pub fn load() -> Result<Self, String> {
        let Some(file) = recent_file() else {
            return Ok(Self::default());
        };
        Self::load_from(&file)
    }

    fn load_from(file: &Path) -> Result<Self, String> {
        if !file.exists() {
            return Ok(Self::default());
        }
        let metadata =
            fs::metadata(file).map_err(|e| format!("Could not read recent projects: {e}"))?;
        if metadata.len() > 1024 * 1024 {
            return Err("Recent projects file exceeds 1 MB.".into());
        }
        let bytes = fs::read(file).map_err(|e| format!("Could not read recent projects: {e}"))?;
        let mut recents: Self = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Could not parse recent projects: {e}"))?;
        recents
            .projects
            .sort_by_key(|project| std::cmp::Reverse(project.opened_at));
        recents.projects.truncate(MAX_RECENT_PROJECTS);
        Ok(recents)
    }

    pub fn record(&mut self, path: &Path) -> Result<(), String> {
        let path = path
            .canonicalize()
            .map_err(|e| format!("Could not resolve project path: {e}"))?;
        if !crate::project::Project::is_project_folder(&path) {
            return Err(format!(
                "Cannot add '{}' to recent projects: no .altgame manifest or scene.json was found.",
                path.display()
            ));
        }
        self.projects
            .retain(|project| !same_path(&project.path, &path));
        self.projects.insert(
            0,
            RecentProject {
                name: path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Untitled project")
                    .to_owned(),
                path,
                opened_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            },
        );
        self.projects.truncate(MAX_RECENT_PROJECTS);
        self.save()
    }

    pub fn remove(&mut self, path: &Path) -> Result<(), String> {
        self.projects
            .retain(|project| !same_path(&project.path, path));
        self.save()
    }

    pub fn clear(&mut self) -> Result<(), String> {
        self.projects.clear();
        self.save()
    }

    fn save(&self) -> Result<(), String> {
        let Some(file) = recent_file() else {
            return Err("Could not locate a writable user data directory.".into());
        };
        self.save_to(&file)
    }

    fn save_to(&self, file: &Path) -> Result<(), String> {
        let parent = file
            .parent()
            .ok_or_else(|| "Invalid recent projects file path.".to_owned())?;
        fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create recent projects folder: {e}"))?;
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| format!("Could not serialize recent projects: {e}"))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|e| format!("Could not create recent projects file: {e}"))?;
        use std::io::Write;
        temporary
            .write_all(&bytes)
            .map_err(|e| format!("Could not write recent projects: {e}"))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|e| format!("Could not flush recent projects: {e}"))?;
        temporary
            .persist(file)
            .map_err(|e| format!("Could not update recent projects: {}", e.error))?;
        Ok(())
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy()),
    }
}

fn recent_file() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    base.map(|base| base.join("Altadra").join("recent-projects.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_project_json_round_trips_and_deduplicates() {
        let recent = RecentProject {
            name: "Example".into(),
            path: PathBuf::from("C:\\Games\\Example"),
            opened_at: 123,
        };
        let json = serde_json::to_string(&recent).unwrap();
        assert_eq!(recent, serde_json::from_str(&json).unwrap());
    }

    #[test]
    fn malformed_recent_project_data_is_reported() {
        assert!(
            serde_json::from_str::<RecentProjects>(
                r#"{"projects":[{"path":"bad","opened_at":0}]}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<RecentProjects>(r#"{"projects":[],"typo":true}"#).is_err());
    }

    #[test]
    fn trims_recent_list_to_configured_limit() {
        let projects = (0..20)
            .map(|index| RecentProject {
                name: index.to_string(),
                path: PathBuf::from(format!("project-{index}")),
                opened_at: index,
            })
            .collect();
        let recent = RecentProjects { projects };
        let json = serde_json::to_string(&recent).unwrap();
        let mut decoded: RecentProjects = serde_json::from_str(&json).unwrap();
        decoded.projects.truncate(MAX_RECENT_PROJECTS);
        assert_eq!(decoded.projects.len(), MAX_RECENT_PROJECTS);
    }

    #[test]
    fn recent_projects_persist_and_reload_in_recency_order() {
        let temporary = tempfile::tempdir().unwrap();
        let file = temporary.path().join("Altadra").join("recent.json");
        let projects = RecentProjects {
            projects: vec![
                RecentProject {
                    name: "Older".into(),
                    path: PathBuf::from("older"),
                    opened_at: 1,
                },
                RecentProject {
                    name: "Newer".into(),
                    path: PathBuf::from("newer"),
                    opened_at: 9,
                },
            ],
        };
        projects.save_to(&file).unwrap();
        let loaded = RecentProjects::load_from(&file).unwrap();
        assert_eq!(loaded.projects[0].name, "Newer");
        assert_eq!(loaded.projects[1].name, "Older");
    }

    #[test]
    fn recent_projects_reject_oversized_and_malformed_files() {
        let temporary = tempfile::tempdir().unwrap();
        let file = temporary.path().join("recent.json");
        std::fs::write(&file, vec![b'x'; 1024 * 1024 + 1]).unwrap();
        assert!(
            RecentProjects::load_from(&file)
                .unwrap_err()
                .contains("exceeds 1 MB")
        );
        std::fs::write(&file, b"{ invalid").unwrap();
        assert!(
            RecentProjects::load_from(&file)
                .unwrap_err()
                .contains("parse recent projects")
        );
    }
}
