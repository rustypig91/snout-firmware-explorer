use super::{AnalysisOptions, Explorer, Loaded, View};
use std::path::PathBuf;

fn preferences_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .map(|p| p.join("snout-firmware-explorer").join("workspace.json"))
}
impl Explorer {
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
        self.job(move || {
            let build = firmware_analysis_core::build::scan_folder(root).map_err(|e| e.to_string())?;
            let mut a = firmware_analysis_core::build::analyze_build_firmware(&build, &path, layout.as_ref())
                .map_err(|e| e.to_string())?;
            let reports = build.artifacts.iter()
                .filter(|a| a.kind == firmware_analysis_core::build::ArtifactKind::StackUsage)
                .map(|a| a.path.clone()).collect();
            let stack = match firmware_analysis_core::stack::analyze_stack_files(&a, reports) {
                Ok(mut s) => {
                    s.warnings.push("Reports may span multiple targets or configurations; check their paths and build ownership.".into());
                    Some(s)
                }
                Err(e) => {
                    a.warnings.push(format!("Stack reports could not be loaded: {e}"));
                    None
                }
            };
            Ok(Loaded::Refresh(Box::new((build, a, stack, layout))))
        });
    }

    pub(super) fn preference_value(&self) -> serde_json::Value {
        serde_json::json!({"version": 1, "folder": self.build.as_ref().map(|b| &b.root), "firmware": self.analysis.as_ref().map(|a| &a.path), "layout": self.layout_override, "layout_source": self.layout_source, "view": self.view.label(), "directories": self.tree, "metric": self.overview_metric.label(), "contributor_ram": self.contributor_ram})
    }
    pub(super) fn save_preferences(&self) -> Result<(), Box<dyn std::error::Error>> {
        let Some(path) = preferences_path() else {
            return Ok(());
        };
        if self.build.is_none() {
            return Ok(());
        }
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, serde_json::to_vec_pretty(&self.preference_value())?)?;
        Ok(())
    }
    pub(super) fn restore_preferences(&mut self) {
        let Some(path) = preferences_path() else {
            return;
        };
        let Ok(data) = std::fs::read(path) else {
            return;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&data) else {
            return;
        };
        self.apply_preferences(&value);
    }
    pub(super) fn apply_preferences(&mut self, value: &serde_json::Value) {
        if value["version"].as_u64() != Some(1) {
            return;
        }
        self.view = View::ALL
            .into_iter()
            .find(|v| Some(v.label()) == value["view"].as_str())
            .unwrap_or(View::Overview);
        self.tree = value["directories"].as_bool().unwrap_or(false);
        self.contributor_ram = value["contributor_ram"].as_bool().unwrap_or(false);
        self.overview_metric = match value["metric"].as_str() {
            Some("RAM") => super::overview::Metric::Ram,
            Some("All sections") => super::overview::Metric::All,
            _ => super::overview::Metric::Flash,
        };
        let Some(folder) = value["folder"]
            .as_str()
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
        else {
            return;
        };
        if let Some(firmware) = value["firmware"]
            .as_str()
            .map(PathBuf::from)
            .filter(|p| p.is_file())
        {
            let layout: Option<AnalysisOptions> = serde_json::from_value(value["layout"].clone())
                .ok()
                .flatten();
            let layout = layout.filter(|l| firmware_analysis_core::validate_options(l).is_ok());
            self.pending_restore = Some((
                firmware,
                layout,
                value["layout_source"]
                    .as_str()
                    .unwrap_or("Saved layout")
                    .into(),
            ));
        }
        self.scan_build(folder);
    }
}
