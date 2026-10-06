use super::{AnalysisOptions, Explorer, Loaded, RememberedFirmware, View};
use std::path::{Path, PathBuf};

pub(super) fn preferences_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .map(|p| p.join("snout-firmware-explorer").join("workspace.json"))
}

fn write_preferences(
    path: &Path,
    value: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Missing preferences folder",
        )
    })?;
    std::fs::create_dir_all(parent)?;
    // Stage alongside the destination so replacement stays on one filesystem.
    // Keep the last saved workspace intact until the new file is complete.
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut staged, value)?;
    staged.as_file().sync_all()?;
    staged.persist(path).map_err(|error| error.error)?;
    Ok(())
}

impl Explorer {
    pub(super) fn open_configuration_folder(&self) -> Result<(), String> {
        let folder = self
            .preferences_file
            .as_ref()
            .and_then(|path| path.parent())
            .ok_or("Configuration folder is unavailable")?;
        std::fs::create_dir_all(folder)
            .map_err(|error| format!("Could not create configuration folder: {error}"))?;
        #[cfg(target_os = "windows")]
        let launcher = "explorer.exe";
        #[cfg(target_os = "macos")]
        let launcher = "open";
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let launcher = "xdg-open";
        let mut child = std::process::Command::new(launcher)
            .arg(folder)
            .spawn()
            .map_err(|error| format!("Could not open configuration folder: {error}"))?;
        // Reap the launcher without blocking the UI while the file explorer runs.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }

    pub(super) fn refresh(&mut self) {
        if self.receiver.is_some() {
            return;
        }
        let Some(build) = &self.build else {
            return;
        };
        let root = build.root.clone();
        let Some(a) = &self.analysis else {
            self.scan_build(root);
            return;
        };
        let path = PathBuf::from(&a.path);
        let layout = self.layout_override.clone();
        let source = layout.as_ref().map(|_| self.layout_source.clone());
        let dependency_map = self.saved_dependency_map(&path);
        let reports = self.saved_stack_selection(&path);
        self.job(move || {
            let build =
                firmware_analysis_core::build::scan_folder(root).map_err(|e| e.to_string())?;
            let (mut a, layout, source) =
                super::workspace::analyze_selected(&build, &path, layout, source)?;
            if let Some(map) = dependency_map {
                super::workspace::read_dependency_map(&mut a, &map);
            }
            let stack = match super::workspace::load_stack_reports(&a, &build, reports) {
                Ok(s) => Some(s),
                Err(e) => {
                    a.warnings
                        .push(format!("Stack reports could not be loaded: {e}"));
                    None
                }
            };
            Ok(Loaded::Refresh(Box::new((build, a, stack, layout, source))))
        });
    }

    pub(super) fn remember_build_folder(&mut self, folder: PathBuf) {
        self.recent_build_folders.retain(|path| path != &folder);
        self.recent_build_folders.insert(0, folder);
        self.recent_build_folders.truncate(5);
    }

    pub(super) fn preference_value(&self) -> serde_json::Value {
        serde_json::json!({"version": 1, "recent_build_folders": self.recent_build_folders, "build_settings": self.build_settings, "check_updates_on_startup": self.updates.check_on_startup, "skipped_version": self.updates.skipped_version, "folder": self.build.as_ref().map(|b| &b.root), "firmware": self.analysis.as_ref().map(|a| &a.path), "layout": self.layout_override, "layout_source": self.layout_source, "view": self.view.label()})
    }
    pub(super) fn save_preferences(&self) -> Result<(), Box<dyn std::error::Error>> {
        let Some(path) = &self.preferences_file else {
            return Ok(());
        };
        write_preferences(path, &self.preference_value())
    }
    pub(super) fn persist_preferences(&mut self) {
        if let Err(error) = self.save_preferences() {
            self.error = Some(format!("Could not save workspace preferences: {error}"));
        }
    }
    pub(super) fn restore_preferences(&mut self, restore_workspace: bool) {
        let Some(path) = &self.preferences_file else {
            return;
        };
        let Ok(data) = std::fs::read(path) else {
            return;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&data) else {
            return;
        };
        if restore_workspace {
            self.apply_preferences(&value);
        } else {
            self.apply_preferences_with_workspace(&value, false);
        }
        // Delete only after reading an authoritative build-settings map. Missing,
        // corrupt or unsupported preferences must never turn into an empty keep-list.
        if value["version"].as_u64() == Some(1)
            && serde_json::from_value::<std::collections::BTreeMap<PathBuf, super::BuildSettings>>(
                value["build_settings"].clone(),
            )
            .is_ok()
        {
            self.cleanup_snapshots();
        }
    }
    pub(super) fn apply_preferences(&mut self, value: &serde_json::Value) {
        self.apply_preferences_with_workspace(value, true);
    }
    pub(super) fn apply_preferences_with_workspace(
        &mut self,
        value: &serde_json::Value,
        restore_workspace: bool,
    ) {
        if value["version"].as_u64() != Some(1) {
            return;
        }
        self.recent_build_folders.clear();
        let recent: Vec<PathBuf> =
            serde_json::from_value(value["recent_build_folders"].clone()).unwrap_or_default();
        for folder in recent {
            let folder = folder.canonicalize().unwrap_or(folder);
            if !self.recent_build_folders.contains(&folder) {
                self.recent_build_folders.push(folder);
            }
            if self.recent_build_folders.len() == 5 {
                break;
            }
        }
        // Older preferences only stored the last open folder.
        if self.recent_build_folders.is_empty() {
            if let Some(folder) = value["folder"].as_str().map(PathBuf::from) {
                let folder = folder.canonicalize().unwrap_or(folder);
                self.remember_build_folder(folder);
            }
        }
        self.build_settings =
            serde_json::from_value(value["build_settings"].clone()).unwrap_or_default();
        self.updates.check_on_startup = value["check_updates_on_startup"].as_bool().unwrap_or(true);
        self.updates.skipped_version = value["skipped_version"].as_str().map(str::to_owned);
        self.change_view(
            View::ALL
                .into_iter()
                .find(|v| Some(v.label()) == value["view"].as_str())
                .unwrap_or(View::Overview),
        );
        let Some(folder) = value["folder"]
            .as_str()
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .and_then(|p| p.canonicalize().ok())
        else {
            return;
        };
        if let Some(firmware) = value["firmware"]
            .as_str()
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .and_then(|p| p.canonicalize().ok())
            .filter(|p| p.starts_with(&folder))
        {
            let layout: Option<AnalysisOptions> = serde_json::from_value(value["layout"].clone())
                .ok()
                .flatten();
            let layout = layout.filter(|l| firmware_analysis_core::validate_options(l).is_ok());
            let settings = self.build_settings.entry(folder.clone()).or_default();
            if settings.firmware.is_none() {
                settings.firmware = Some(firmware.clone());
            }
            if let Some(options) = &layout {
                settings
                    .layouts
                    .entry(firmware.clone())
                    .or_insert_with(|| super::SavedLayout {
                        options: options.clone(),
                        source: value["layout_source"]
                            .as_str()
                            .unwrap_or("Saved layout")
                            .into(),
                    });
            }
            self.remembered_firmware = Some(RememberedFirmware {
                folder: folder.clone(),
                path: firmware,
                layout,
                source: value["layout_source"]
                    .as_str()
                    .unwrap_or("Saved layout")
                    .into(),
            });
        }
        if restore_workspace {
            self.scan_build(folder);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::io::Read;

    #[test]
    fn saves_replace_complete_preferences_without_truncating_the_previous_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config/workspace.json");
        let first = serde_json::json!({"version": 1, "firmware": "first.elf"});
        write_preferences(&path, &first).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap(),
            first
        );
        // Unix permits replacing a file while a reader holds its old inode.
        // Windows replacement requires the destination to be closed.
        #[cfg(unix)]
        let mut previous = std::fs::File::open(&path).unwrap();
        let next = serde_json::json!({"version": 1, "firmware": "next.elf"});
        write_preferences(&path, &next).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap(),
            next
        );
        #[cfg(unix)]
        {
            let mut old_data = Vec::new();
            previous.read_to_end(&mut old_data).unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&old_data).unwrap(),
                first
            );
        }
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn failed_replacement_preserves_the_destination_and_removes_staging_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("keep"), b"saved").unwrap();
        assert!(write_preferences(&path, &serde_json::json!({"version": 1})).is_err());
        assert_eq!(std::fs::read(path.join("keep")).unwrap(), b"saved");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
