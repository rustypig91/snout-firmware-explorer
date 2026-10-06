use super::{egui, Analysis, Explorer, StackReport};
use firmware_analysis_core::{format_bytes, FileTree, Symbol};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct SnapshotStore {
    // Snapshot reports live in separate files; baseline selection is session-only.
    #[serde(skip)]
    snapshots: Vec<Snapshot>,
    #[serde(skip)]
    active: BTreeMap<String, String>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Snapshot {
    name: String,
    firmware: String,
    taken_at: chrono::DateTime<chrono::Utc>,
    analysis: Analysis,
    stack: Option<StackReport>,
    values: BTreeMap<String, u64>,
}
impl Snapshot {
    fn time_label(&self) -> String {
        format!(
            "Taken: {}",
            self.taken_at
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S %:z")
        )
    }
}

// Keep deletion separate from the selection hit target, including when details wrap.
fn snapshot_row(ui: &mut egui::Ui, snapshot: &Snapshot, active: bool) -> (bool, bool) {
    let accent = super::views::ACCENT;
    let mut compare = false;
    let mut delete = false;
    egui::Frame::none().inner_margin(12.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            let body = ui.allocate_ui_with_layout(
                egui::vec2((ui.available_width() - 40.0).max(1.0), 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 5.0;
                    ui.label(
                        egui::RichText::new(&snapshot.name)
                            .strong()
                            .size(ui.text_style_height(&egui::TextStyle::Body) + 1.0),
                    );
                    ui.label(format!(
                        "ELF: {}",
                        super::display::display_path(&snapshot.firmware)
                    ));
                    ui.label(snapshot.time_label());
                    if active {
                        ui.label(egui::RichText::new("Current baseline").color(accent));
                    }
                },
            );
            let response = ui.interact(
                body.response.rect,
                ui.id().with("select"),
                egui::Sense::click(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::SelectableLabel,
                    ui.is_enabled(),
                    active,
                    &snapshot.name,
                )
            });
            if response.hovered() || response.has_focus() {
                ui.painter().rect_stroke(
                    response.rect.expand(4.0),
                    4.0,
                    egui::Stroke::new(1.0_f32, accent),
                );
            }
            compare = response
                .on_hover_text("Compare loaded firmware with this snapshot")
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked();

            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    format!("Delete snapshot {}", snapshot.name),
                )
            });
            let color = if response.hovered() || response.has_focus() {
                ui.painter()
                    .rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
                ui.visuals().error_fg_color
            } else {
                ui.visuals().text_color()
            };
            let stroke = egui::Stroke::new(1.5_f32, color);
            let center = rect.center();
            let bin =
                egui::Rect::from_center_size(center + egui::vec2(0.0, 2.0), egui::vec2(10.0, 12.0));
            ui.painter().rect_stroke(bin, 1.0, stroke);
            ui.painter().line_segment(
                [
                    center + egui::vec2(-7.0, -6.0),
                    center + egui::vec2(7.0, -6.0),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    center + egui::vec2(-3.0, -9.0),
                    center + egui::vec2(3.0, -9.0),
                ],
                stroke,
            );
            for x in [-2.0, 2.0] {
                ui.painter().line_segment(
                    [center + egui::vec2(x, -1.0), center + egui::vec2(x, 5.0)],
                    stroke,
                );
            }
            delete = response
                .on_hover_text(format!("Delete snapshot “{}”", snapshot.name))
                .clicked();
        });
    });
    (compare, delete)
}
#[derive(serde::Serialize, serde::Deserialize)]
struct SnapshotFile {
    version: u32,
    build_folder: PathBuf,
    snapshot: Snapshot,
}
fn component(identity: &str) -> String {
    // Three components share the Windows path budget with the configuration
    // directory. Keep 128 bits of identity and store display names in the JSON.
    format!("{:x}", Sha256::digest(identity.as_bytes()))[..32].into()
}
fn build_directory(preferences: &Path, root: &Path) -> Result<PathBuf, String> {
    let parent = preferences
        .parent()
        .ok_or("Missing user configuration directory")?;
    let identity = root.to_string_lossy();
    Ok(parent.join("snapshots").join(component(&identity)))
}
fn snapshot_path(preferences: &Path, root: &Path, snapshot: &Snapshot) -> Result<PathBuf, String> {
    Ok(build_directory(preferences, root)?
        .join(component(&snapshot.firmware))
        .join(format!("{}.json", component(&snapshot.name))))
}
fn write_snapshot(
    preferences: &Path,
    root: &Path,
    snapshot: &Snapshot,
    overwrite: bool,
) -> Result<(), String> {
    let path = snapshot_path(preferences, root, snapshot)?;
    let value = serde_json::to_value(SnapshotFile {
        version: 1,
        build_folder: root.into(),
        snapshot: snapshot.clone(),
    })
    .map_err(|e| e.to_string())?;
    let parent = path.parent().unwrap();
    std::fs::create_dir_all(parent).map_err(|e| {
        format!(
            "Could not create snapshot directory {}: {e}",
            parent.display()
        )
    })?;
    // Rust canonicalization supplies a verbatim (extended-length) path on Windows.
    // tempfile passes paths straight to Win32 for both staging and persistence,
    // so both must use this directory even when APPDATA itself is very long.
    let parent = std::fs::canonicalize(parent).map_err(|e| {
        format!(
            "Could not resolve snapshot directory {}: {e}",
            parent.display()
        )
    })?;
    let destination = parent.join(path.file_name().unwrap());
    let mut staged = tempfile::NamedTempFile::new_in(&parent)
        .map_err(|e| format!("Could not stage snapshot in {}: {e}", parent.display()))?;
    serde_json::to_writer_pretty(&mut staged, &value).map_err(|e| e.to_string())?;
    staged.as_file().sync_all().map_err(|e| e.to_string())?;
    let saved = if overwrite {
        staged.persist(&destination)
    } else {
        staged.persist_noclobber(&destination)
    };
    saved.map_err(|e| format!("Could not save snapshot to {}: {}", path.display(), e.error))?;
    Ok(())
}
#[derive(Clone)]
pub(super) enum Dialog {
    Manager,
    Overwrite(String),
    Delete(String),
    Saving(String),
}
pub(super) struct SaveJob {
    receiver: std::sync::mpsc::Receiver<Result<Snapshot, String>>,
    presented: bool,
}
pub(super) fn symbol_key(s: &Symbol) -> String {
    // Never include placement or source line: both can change after a rebuild.
    serde_json::to_string(&(&s.name, &s.kind, &s.source_file, &s.compilation_unit)).unwrap()
}
pub(super) fn placement_field(region: &str, placement: &str) -> String {
    serde_json::to_string(&(region, placement)).unwrap()
}
pub(super) fn stack_key(e: &firmware_analysis_core::stack::StackEntry) -> String {
    serde_json::to_string(&(&e.report_file, &e.source_file, &e.function)).unwrap()
}
fn key(domain: &str, id: &str, field: &str) -> String {
    serde_json::to_string(&(domain, id, field)).unwrap()
}
fn collect(a: &Analysis, stack: Option<&StackReport>) -> BTreeMap<String, u64> {
    fn fields(
        values: &mut BTreeMap<String, u64>,
        domain: &str,
        id: &str,
        value: serde_json::Value,
        prefix: &str,
    ) {
        if let Some(v) = value.as_u64() {
            values.insert(key(domain, id, prefix), v);
        } else if let serde_json::Value::Object(object) = value {
            for (field, value) in object {
                let field = if prefix.is_empty() {
                    field
                } else {
                    format!("{prefix}.{field}")
                };
                fields(values, domain, id, value, &field);
            }
        }
    }
    fn add<T: serde::Serialize>(v: &mut BTreeMap<String, u64>, d: &str, id: &str, item: &T) {
        fields(v, d, id, serde_json::to_value(item).unwrap(), "");
    }
    fn add_unique<T: serde::Serialize>(
        v: &mut BTreeMap<String, u64>,
        seen: &mut HashSet<(String, String)>,
        domain: &str,
        id: &str,
        item: &T,
    ) {
        if seen.insert((domain.into(), id.into())) {
            add(v, domain, id, item);
        } else {
            v.insert(key(domain, id, "ambiguous"), 1);
        }
    }
    fn tree(v: &mut BTreeMap<String, u64>, node: &FileTree, path: &str) {
        add(v, "tree", path, &node.usage);
        for child in &node.children {
            let path = if path.is_empty() {
                child.name.clone()
            } else {
                format!("{path}/{}", child.name)
            };
            tree(v, child, &path);
        }
    }
    let mut v = BTreeMap::new();
    let mut seen = HashSet::new();
    add(&mut v, "totals", "", &a.totals);
    add(&mut v, "metadata", "", &a.metadata);
    add(&mut v, "unattributed", "", &a.unattributed);
    add(
        &mut v,
        "counts",
        "",
        &serde_json::json!({
            "symbols": a.symbols.len(), "files": a.files.iter().filter(|f| f.path != "[unattributed]").count(),
            "tls": a.tls.as_ref().map_or(0,|t| t.symbols.len()),
            "stack": stack.map_or(0,|s| s.entries.len()),
            "dependency_nodes": a.dependencies.nodes.len(), "dependency_edges": a.dependencies.edges.len()
        }),
    );
    if let Some(tls) = &a.tls {
        add(&mut v, "tls", "", tls);
        for s in &tls.symbols {
            add_unique(&mut v, &mut seen, "tls_symbol", &s.name, s);
        }
    }
    for f in &a.files {
        add(&mut v, "file", &f.path, f);
    }
    for s in &a.sections {
        add_unique(&mut v, &mut seen, "section", &s.name, s);
        for (ram, metric) in [(false, "flash"), (true, "ram")] {
            let (gap, uncovered) = super::insights::section_unattributed(a, s, ram);
            for (field, value) in [
                ("unattributed", gap),
                ("uncovered", uncovered),
                ("unowned", gap - uncovered),
            ] {
                v.insert(key("section", &s.name, &format!("{field}.{metric}")), value);
            }
        }
    }
    let mut symbols = BTreeMap::<String, Vec<&Symbol>>::new();
    for s in &a.symbols {
        symbols.entry(symbol_key(s)).or_default().push(s);
    }
    for (id, matches) in symbols {
        if matches.len() == 1 {
            add(&mut v, "symbol", &id, matches[0]);
        } else {
            v.insert(key("symbol", &id, "ambiguous"), 1);
        }
    }
    for r in &a.memory_map {
        let id = serde_json::to_string(&(&r.name, &r.space)).unwrap();
        add_unique(&mut v, &mut seen, "range", &id, r);
        v.insert(key("range", &id, "end"), r.address.saturating_add(r.size));
    }
    for r in &a.options.regions {
        add_unique(&mut v, &mut seen, "region", &r.name, r);
        let usage = firmware_analysis_core::regions::region_usage(a, r);
        v.insert(key("region", &r.name, "used"), usage.used);
        v.insert(key("region", &r.name, "free"), usage.free);
        for entry in &usage.symbols {
            let symbol = &a.symbols[entry.symbol_index];
            v.insert(
                key(
                    "symbol",
                    &symbol_key(symbol),
                    &placement_field(&r.name, entry.placement),
                ),
                entry.address,
            );
        }
        v.insert(
            key("region", &r.name, "end"),
            r.start.saturating_add(r.size),
        );
    }
    tree(&mut v, &a.tree, "");
    let mut roles = BTreeMap::<&str, u64>::new();
    for s in &a.sections {
        *roles.entry(super::insights::ram_role(s)).or_default() += s.usage.ram;
    }
    for (role, size) in roles {
        v.insert(key("ram_role", role, "size"), size);
    }
    for n in &a.dependencies.nodes {
        if let Some(u) = n.usage {
            add(&mut v, "dependency", &n.id, &u);
        }
    }
    if let Some(stack) = stack {
        for e in &stack.entries {
            add_unique(&mut v, &mut seen, "stack", &stack_key(e), e);
        }
    }
    v
}
fn annotate(current: String, value: u64, old: Option<u64>, address: bool) -> String {
    match old {
        Some(old) if old != value => {
            let delta = i128::from(value) - i128::from(old);
            let sign = if delta < 0 { '-' } else { '+' };
            let amount = if address {
                format!("0x{:x}", delta.unsigned_abs())
            } else {
                format_bytes(delta.unsigned_abs() as u64)
            };
            format!("{current} ({sign}{amount})")
        }
        _ => current,
    }
}
impl Explorer {
    fn snapshot_firmware(&self) -> Option<String> {
        let path = std::path::Path::new(&self.analysis.as_ref()?.path);
        Some(
            path.strip_prefix(&self.build.as_ref()?.root)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned(),
        )
    }
    fn selected_snapshot(&self) -> Option<&Snapshot> {
        let firmware = self.snapshot_firmware()?;
        let name = self.snapshots.active.get(&firmware)?;
        self.snapshots
            .snapshots
            .iter()
            .find(|s| s.firmware == firmware && &s.name == name)
    }
    pub(super) fn sync_snapshot_comparison(&mut self) {
        self.comparison = self
            .snapshot_analysis()
            .zip(self.analysis.as_deref())
            .map(|(baseline, current)| super::compare(baseline, current));
        self.table_cache = Default::default();
    }
    pub(super) fn snapshot_analysis(&self) -> Option<&Analysis> {
        Some(&self.selected_snapshot()?.analysis)
    }
    pub(super) fn snapshot_label(&self) -> Option<&str> {
        Some(&self.selected_snapshot()?.name)
    }
    pub(super) fn snapshot_old(&self, domain: &str, id: &str, field: &str) -> Option<u64> {
        let snapshot = self.selected_snapshot()?;
        if snapshot.values.contains_key(&key(domain, id, "ambiguous")) {
            return None;
        }
        snapshot.values.get(&key(domain, id, field)).copied()
    }
    pub(super) fn snapshot_bytes(&self, domain: &str, id: &str, field: &str, value: u64) -> String {
        if let Some(snapshot) = self.selected_snapshot() {
            if snapshot.values.contains_key(&key(domain, id, "ambiguous")) {
                return format!("{} (baseline ambiguous)", format_bytes(value));
            }
            if !snapshot.values.contains_key(&key(domain, id, field)) {
                return format!("{} (new)", format_bytes(value));
            }
        }
        annotate(
            format_bytes(value),
            value,
            self.snapshot_old(domain, id, field),
            false,
        )
    }
    pub(super) fn snapshot_address(
        &self,
        domain: &str,
        id: &str,
        field: &str,
        value: u64,
    ) -> String {
        if let Some(snapshot) = self.selected_snapshot() {
            if snapshot.values.contains_key(&key(domain, id, "ambiguous")) {
                return format!("{value:#010x} (baseline ambiguous)");
            }
            if !snapshot.values.contains_key(&key(domain, id, field)) {
                return format!("{value:#010x} (new)");
            }
        }
        annotate(
            format!("{value:#010x}"),
            value,
            self.snapshot_old(domain, id, field),
            true,
        )
    }
    pub(super) fn snapshot_count(&self, domain: &str, id: &str, field: &str, value: u64) -> String {
        if self
            .selected_snapshot()
            .is_some_and(|snapshot| snapshot.values.contains_key(&key(domain, id, "ambiguous")))
        {
            return format!("{value} (baseline ambiguous)");
        }
        match self.snapshot_old(domain, id, field) {
            Some(old) if old != value => {
                format!("{value} ({:+})", i128::from(value) - i128::from(old))
            }
            _ if self.selected_snapshot().is_some()
                && self.snapshot_old(domain, id, field).is_none() =>
            {
                format!("{value} (new)")
            }
            _ => value.to_string(),
        }
    }
    pub(super) fn snapshot_placement_address(
        &self,
        symbol: &Symbol,
        region: &str,
        placement: &str,
        value: u64,
    ) -> String {
        if self.selected_snapshot().is_some_and(|snapshot| {
            snapshot
                .values
                .contains_key(&key("region", region, "ambiguous"))
        }) {
            return format!("{value:#010x} (baseline ambiguous)");
        }
        self.snapshot_address(
            "symbol",
            &symbol_key(symbol),
            &placement_field(region, placement),
            value,
        )
    }
    pub(super) fn snapshot_percentage(&self, value: f64, old: Option<f64>) -> String {
        let current = format!("{value:.1}%");
        match old.filter(|_| self.selected_snapshot().is_some()) {
            Some(old) if (value - old).abs() >= 0.05 => {
                format!("{current} ({:+.1} pp)", value - old)
            }
            _ => current,
        }
    }
    pub(super) fn snapshot_region_percentage(&self, name: &str, used: u64, size: u64) -> String {
        let old = self
            .snapshot_old("region", name, "used")
            .zip(self.snapshot_old("region", name, "size"))
            .map(|(used, size)| used as f64 * 100.0 / size.max(1) as f64);
        self.snapshot_percentage(used as f64 * 100.0 / size.max(1) as f64, old)
    }
    pub(super) fn snapshot_difference(&self, value: u64, old: Option<u64>) -> String {
        if self.selected_snapshot().is_some() {
            annotate(format_bytes(value), value, old, false)
        } else {
            format_bytes(value)
        }
    }
    pub(super) fn cleanup_snapshots(&mut self) {
        let Some(preferences) = &self.preferences_file else {
            return;
        };
        let Some(parent) = preferences.parent() else {
            return;
        };
        let directory = parent.join("snapshots");
        let retained: HashSet<_> = self
            .build_settings
            .keys()
            .filter_map(|root| build_directory(preferences, root).ok())
            .collect();
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                self.snapshot_error = Some(format!("Could not clean up snapshots: {error}"));
                return;
            }
        };
        let mut errors = Vec::new();
        for entry in entries {
            let result = (|| -> Result<(), String> {
                let entry = entry.map_err(|error| error.to_string())?;
                let path = entry.path();
                // Inspect the entry itself so cleanup never follows directory symlinks.
                if entry
                    .file_type()
                    .map_err(|error| error.to_string())?
                    .is_dir()
                    && !retained.contains(&path)
                {
                    std::fs::remove_dir_all(&path)
                        .map_err(|error| format!("{}: {error}", path.display()))?;
                }
                Ok(())
            })();
            if let Err(error) = result {
                errors.push(error);
            }
        }
        if !errors.is_empty() {
            self.snapshot_error = Some(format!(
                "Could not clean up snapshots: {}",
                errors.join("; ")
            ));
        }
    }
    pub(super) fn load_snapshots(&mut self) {
        self.snapshots = self
            .build
            .as_ref()
            .and_then(|build| self.build_settings.get(&build.root))
            .map(|settings| settings.snapshots.clone())
            .unwrap_or_default();
        self.snapshot_dialog = None;
        let (Some(preferences), Some(build)) = (&self.preferences_file, &self.build) else {
            return;
        };
        let directory = match build_directory(preferences, &build.root) {
            Ok(path) => path,
            Err(error) => {
                self.snapshot_error = Some(error);
                return;
            }
        };
        let read =
            (|| -> Result<(), String> {
                let entries = match std::fs::read_dir(&directory) {
                    Ok(entries) => entries,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                    Err(error) => return Err(error.to_string()),
                };
                for entry in entries {
                    let folder = entry.map_err(|e| e.to_string())?.path();
                    if !folder.is_dir() {
                        continue;
                    }
                    for entry in std::fs::read_dir(folder).map_err(|e| e.to_string())? {
                        let path = entry.map_err(|e| e.to_string())?.path();
                        if path.extension().is_none_or(|ext| ext != "json") {
                            continue;
                        }
                        let loaded = (|| -> Result<Snapshot, String> {
                            let mut stored: SnapshotFile = serde_json::from_slice(
                                &std::fs::read(&path).map_err(|e| e.to_string())?,
                            )
                            .map_err(|e| e.to_string())?;
                            if stored.version != 1 || stored.build_folder != build.root {
                                return Err("Unsupported snapshot version or build folder".into());
                            }
                            if snapshot_path(preferences, &build.root, &stored.snapshot)? != path {
                                return Err("Snapshot identity does not match its file path".into());
                            }
                            // Rebuild derived lookup data so reports saved by older versions
                            // also benefit from identity and ambiguity fixes.
                            stored.snapshot.values =
                                collect(&stored.snapshot.analysis, stored.snapshot.stack.as_ref());
                            Ok(stored.snapshot)
                        })();
                        match loaded {
                            Ok(snapshot) => {
                                if !self.snapshots.snapshots.iter().any(|s| {
                                    s.firmware == snapshot.firmware && s.name == snapshot.name
                                }) {
                                    self.snapshots.snapshots.push(snapshot);
                                }
                            }
                            Err(error) => {
                                self.snapshot_error = Some(format!(
                                    "Could not load snapshot {}: {error}",
                                    path.display()
                                ))
                            }
                        }
                    }
                }
                Ok(())
            })();
        if let Err(error) = read {
            self.snapshot_error = Some(format!("Could not load snapshots: {error}"));
        }
        self.snapshots.snapshots.sort_by(|a, b| {
            a.firmware
                .cmp(&b.firmware)
                .then_with(|| a.name.cmp(&b.name))
        });
    }
    fn remember_snapshot_selection(
        &mut self,
        active: &BTreeMap<String, String>,
    ) -> Result<(), String> {
        let root = self
            .build
            .as_ref()
            .ok_or("Select a build folder first")?
            .root
            .clone();
        self.build_settings
            .entry(root)
            .or_default()
            .snapshots
            .active = active.clone();
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn take_snapshot(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Enter a snapshot name.".into());
        }
        let firmware = self.snapshot_firmware().ok_or("Select firmware first")?;
        if self
            .snapshots
            .snapshots
            .iter()
            .any(|s| s.name == name && s.firmware == firmware)
        {
            return Err("A snapshot with that name already exists for this firmware.".into());
        }
        let analysis = self.analysis.as_ref().unwrap();
        let snapshot = Snapshot {
            name: name.into(),
            firmware,
            taken_at: chrono::Utc::now(),
            values: collect(analysis, self.stack.as_ref()),
            analysis: (**analysis).clone(),
            stack: self.stack.clone(),
        };
        let preferences = self
            .preferences_file
            .as_ref()
            .ok_or("Workspace preferences are unavailable; the snapshot cannot be saved.")?;
        let root = &self
            .build
            .as_ref()
            .ok_or("Select a build folder first")?
            .root;
        write_snapshot(preferences, root, &snapshot, false)?;
        self.snapshots.snapshots.push(snapshot);
        Ok(())
    }
    pub(super) fn select_snapshot(&mut self, name: Option<String>) -> Result<(), String> {
        let firmware = self.snapshot_firmware().ok_or("Select firmware first")?;
        let mut active = self.snapshots.active.clone();
        if let Some(name) = name {
            if !self
                .snapshots
                .snapshots
                .iter()
                .any(|s| s.firmware == firmware && s.name == name)
            {
                return Err("The selected snapshot does not exist for this firmware.".into());
            }
            active.insert(firmware, name);
        } else {
            active.remove(&firmware);
        }
        self.remember_snapshot_selection(&active)?;
        self.snapshots.active = active;
        self.sync_snapshot_comparison();
        self.report_revision = self.report_revision.wrapping_add(1);
        self.details = None;
        Ok(())
    }
    pub(super) fn open_snapshot_manager(&mut self) {
        self.snapshot_dialog = Some(Dialog::Manager);
        self.snapshot_name.clear();
        self.snapshot_dialog_error = None;
        self.snapshot_message = None;
    }
    fn request_snapshot_save(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let name = self.snapshot_name.trim().to_owned();
        if name.is_empty() {
            return Err("Enter a snapshot name.".into());
        }
        let firmware = self.snapshot_firmware().ok_or("Select firmware first")?;
        let preferences = self
            .preferences_file
            .as_ref()
            .ok_or("Snapshot storage is unavailable")?;
        let root = &self
            .build
            .as_ref()
            .ok_or("Select a build folder first")?
            .root;
        // Check disk too, so a damaged or externally created file is never silently replaced.
        let path = build_directory(preferences, root)?
            .join(component(&firmware))
            .join(format!("{}.json", component(&name)));
        self.snapshot_dialog_error = None;
        self.snapshot_message = None;
        if path.exists()
            || self
                .snapshots
                .snapshots
                .iter()
                .any(|s| s.firmware == firmware && s.name == name)
        {
            self.snapshot_dialog = Some(Dialog::Overwrite(name));
            Ok(())
        } else {
            self.begin_snapshot_save(name, false, ctx)
        }
    }
    fn begin_snapshot_save(
        &mut self,
        name: String,
        overwrite: bool,
        ctx: &egui::Context,
    ) -> Result<(), String> {
        if self.snapshot_job.is_some() || self.receiver.is_some() {
            return Err("Wait for the current operation to finish.".into());
        }
        let firmware = self.snapshot_firmware().ok_or("Select firmware first")?;
        let analysis = self.analysis.clone().ok_or("Select firmware first")?;
        let stack = self.stack.clone();
        let preferences = self
            .preferences_file
            .clone()
            .ok_or("Snapshot storage is unavailable")?;
        let root = self
            .build
            .as_ref()
            .ok_or("Select a build folder first")?
            .root
            .clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let context = ctx.clone();
        let saved_name = name.clone();
        let taken_at = chrono::Utc::now();
        std::thread::spawn(move || {
            let snapshot = Snapshot {
                name: saved_name,
                firmware,
                taken_at,
                values: collect(&analysis, stack.as_ref()),
                analysis: (*analysis).clone(),
                stack,
            };
            let result =
                write_snapshot(&preferences, &root, &snapshot, overwrite).map(|()| snapshot);
            let _ = sender.send(result);
            context.request_repaint();
        });
        self.snapshot_job = Some(SaveJob {
            receiver,
            presented: false,
        });
        self.snapshot_dialog_error = None;
        self.snapshot_dialog = Some(Dialog::Saving(name));
        ctx.request_repaint();
        Ok(())
    }
    fn poll_snapshot_save(&mut self) {
        let Some(job) = &self.snapshot_job else {
            return;
        };
        if !job.presented {
            return;
        }
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("The snapshot worker stopped unexpectedly.".into())
            }
        };
        self.snapshot_job = None;
        self.snapshot_dialog = Some(Dialog::Manager);
        match result {
            Ok(snapshot) => {
                self.snapshot_message = Some(format!("Saved snapshot “{}”.", snapshot.name));
                self.snapshots
                    .snapshots
                    .retain(|s| s.firmware != snapshot.firmware || s.name != snapshot.name);
                self.snapshots.snapshots.push(snapshot);
                self.snapshots.snapshots.sort_by(|a, b| a.name.cmp(&b.name));
                self.sync_snapshot_comparison();
                self.report_revision = self.report_revision.wrapping_add(1);
                self.details = None;
            }
            Err(error) => self.snapshot_dialog_error = Some(error),
        }
    }
    fn delete_snapshot(&mut self, name: &str) -> Result<(), String> {
        let firmware = self.snapshot_firmware().ok_or("Select firmware first")?;
        let snapshot = self
            .snapshots
            .snapshots
            .iter()
            .find(|s| s.firmware == firmware && s.name == name)
            .ok_or("The snapshot no longer exists")?;
        let path = snapshot_path(
            self.preferences_file
                .as_ref()
                .ok_or("Snapshot storage is unavailable")?,
            &self
                .build
                .as_ref()
                .ok_or("Select a build folder first")?
                .root,
            snapshot,
        )?;
        let active = self.snapshot_label() == Some(name);
        if active {
            self.select_snapshot(None)?;
        }
        if let Err(error) = std::fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                if active {
                    if let Err(restore) = self.select_snapshot(Some(name.into())) {
                        return Err(format!("Could not delete snapshot: {error}. Could not restore baseline selection: {restore}"));
                    }
                }
                return Err(format!("Could not delete snapshot: {error}"));
            }
        }
        self.snapshots
            .snapshots
            .retain(|s| s.firmware != firmware || s.name != name);
        self.report_revision = self.report_revision.wrapping_add(1);
        self.details = None;
        Ok(())
    }
    pub(super) fn show_snapshot_dialog(&mut self, ctx: &egui::Context) {
        self.poll_snapshot_save();
        let Some(dialog) = self.snapshot_dialog.clone() else {
            return;
        };
        enum Action {
            Save,
            Overwrite(String),
            AskDelete(String),
            Delete(String),
            Compare(String),
            Stop,
            Back,
            Close,
        }
        let mut action = None;
        let width = (ctx.screen_rect().width() - 64.0).clamp(240.0, 520.0);
        let id = egui::Id::new("snapshot_manager");
        let mut area = egui::Modal::default_area(id);
        if let Some(rect) = ctx.memory(|memory| memory.area_rect(id)) {
            // Tiny layout differences can put the centered origin on opposite
            // sides of a half-pixel boundary at fractional display scales.
            // Discard layout noise below 1/64 pixel before Area rounds its position.
            let scale = ctx.pixels_per_point();
            let size = (rect.size() * scale * 64.0).round() / (scale * 64.0);
            area = area.anchor(
                egui::Align2::LEFT_TOP,
                (ctx.screen_rect().size() - size) * 0.5,
            );
        }
        let response = egui::Modal::new(id)
            .area(area)
            .backdrop_color(egui::Color32::from_black_alpha(160))
            .frame(egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(16.0)))
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.style_mut().text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
                ui.spacing_mut().item_spacing = egui::vec2(10.0,12.0);
                ui.spacing_mut().button_padding = egui::vec2(10.0,5.0);
                ui.spacing_mut().interact_size.y = 28.0;
                let visuals = ui.visuals_mut();
                visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(49,56,66);
                visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32,egui::Color32::from_rgb(87,99,114));
                visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(61,75,92);
                visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32,super::views::ACCENT);
                ui.heading("Snapshots");
                ui.separator();
                if let Some(error) = &self.snapshot_dialog_error {
                    ui.colored_label(egui::Color32::LIGHT_RED,error);
                }
                match &dialog {
                    Dialog::Saving(name) => {
                        ui.horizontal(|ui| { ui.spinner(); ui.label(format!("Saving snapshot “{name}”…")); });
                        ui.label("Please wait while the snapshot is saved.");
                        ctx.request_repaint_after(std::time::Duration::from_millis(50));
                    }
                    Dialog::Overwrite(name) => {
                        ui.label(format!("A snapshot named “{name}” already exists. Overwrite it with the currently loaded firmware?"));
                        ui.horizontal(|ui| {
                            if ui.button("Overwrite").clicked() { action = Some(Action::Overwrite(name.clone())); }
                            if ui.button("Cancel").clicked() { action = Some(Action::Back); }
                        });
                    }
                    Dialog::Delete(name) => {
                        ui.label(format!("Delete snapshot “{name}”?"));
                        if self.snapshot_label() == Some(name.as_str()) { ui.label("This also stops the current baseline comparison."); }
                        ui.horizontal(|ui| {
                            if ui.button("Delete snapshot").clicked() { action = Some(Action::Delete(name.clone())); }
                            if ui.button("Cancel").clicked() { action = Some(Action::Back); }
                        });
                    }
                    Dialog::Manager => {
                        if let Some(message) = &self.snapshot_message { ui.label(message); }
                        ui.label("Save the currently loaded firmware as a baseline.");
                        let name_response = ui.horizontal(|ui| {
                            ui.label("Snapshot name:");
                            ui.add(egui::TextEdit::singleline(&mut self.snapshot_name)
                                .hint_text("e.g. Before adding Bluetooth").desired_width(ui.available_width()))
                        }).inner;
                        if name_response.gained_focus() || (self.snapshot_name.is_empty() && !ctx.wants_keyboard_input()) { name_response.request_focus(); }
                        let can_save = !self.snapshot_name.trim().is_empty() && self.receiver.is_none();
                        if can_save && name_response.lost_focus() && ui.input(|input|input.key_pressed(egui::Key::Enter)) { action = Some(Action::Save); }
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                                if ui.add_enabled(can_save,egui::Button::new("Save snapshot")
                                    .fill(egui::Color32::from_rgb(43,91,122))
                                    .stroke(egui::Stroke::new(1.0_f32,super::views::ACCENT))).clicked() { action = Some(Action::Save); }
                            });
                        });
                        ui.separator();
                        let firmware = self.snapshot_firmware();
                        let mut snapshots: Vec<_> = self.snapshots.snapshots.iter().filter(|s|firmware.as_ref() == Some(&s.firmware)).collect();
                        snapshots.sort_by(|a,b| b.taken_at.cmp(&a.taken_at).then_with(|| a.name.cmp(&b.name)));
                        ui.strong(format!("Saved snapshots ({})", snapshots.len()));
                        if snapshots.is_empty() { ui.label("No snapshots for this firmware yet."); }
                        else {
                            ui.label("Select a snapshot to compare with the loaded firmware.");
                            egui::ScrollArea::vertical().max_height((ctx.screen_rect().height() - 360.0).clamp(100.0, 340.0)).auto_shrink([false,true]).show(ui,|ui| {
                                for snapshot in snapshots {
                                    ui.push_id((&snapshot.firmware, &snapshot.name), |ui| {
                                        let (compare, delete) = snapshot_row(ui, snapshot, self.snapshot_label() == Some(snapshot.name.as_str()));
                                        if compare { action = Some(Action::Compare(snapshot.name.clone())); }
                                        if delete { action = Some(Action::AskDelete(snapshot.name.clone())); }
                                    });
                                }
                            });
                        }
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                                if ui.button("Close").clicked() { action = Some(Action::Close); }
                                if self.snapshot_label().is_some() && ui.button("Stop comparing").clicked() { action = Some(Action::Stop); }
                            });
                        });
                    }
                }
            });
        if matches!(dialog, Dialog::Saving(_)) {
            if let Some(job) = &mut self.snapshot_job {
                job.presented = true;
            }
        }
        if !matches!(dialog, Dialog::Saving(_)) && response.should_close() && action.is_none() {
            action = Some(if matches!(dialog, Dialog::Manager) {
                Action::Close
            } else {
                Action::Back
            });
        }
        let result = match action {
            Some(Action::Save) => self.request_snapshot_save(ctx),
            Some(Action::Overwrite(name)) => self.begin_snapshot_save(name, true, ctx),
            Some(Action::AskDelete(name)) => {
                self.snapshot_dialog = Some(Dialog::Delete(name));
                self.snapshot_dialog_error = None;
                Ok(())
            }
            Some(Action::Delete(name)) => {
                let result = self.delete_snapshot(&name);
                self.snapshot_dialog = Some(Dialog::Manager);
                if result.is_ok() {
                    self.snapshot_message = Some(format!("Deleted snapshot “{name}”."));
                }
                result
            }
            Some(Action::Compare(name)) => {
                let result = self.select_snapshot(Some(name));
                if result.is_ok() {
                    self.snapshot_dialog = None;
                }
                result
            }
            Some(Action::Stop) => {
                let result = self.select_snapshot(None);
                if result.is_ok() {
                    self.snapshot_message = Some("Baseline comparison stopped.".into());
                }
                result
            }
            Some(Action::Back) => {
                self.snapshot_dialog = Some(Dialog::Manager);
                self.snapshot_dialog_error = None;
                Ok(())
            }
            Some(Action::Close) => {
                self.snapshot_dialog = None;
                Ok(())
            }
            None => Ok(()),
        };
        if let Err(error) = result {
            self.snapshot_dialog_error = Some(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{mpsc, Arc},
        time::Duration,
    };

    fn finish(app: &mut Explorer) {
        while let Some(receiver) = app.receiver.take() {
            let result = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
            let (tx, rx) = mpsc::channel();
            tx.send(result).unwrap();
            app.receiver = Some(rx);
            app.poll();
        }
    }
    fn open(root: &std::path::Path) -> Explorer {
        let mut app = Explorer {
            preferences_file: Some(root.parent().unwrap().join("workspace.json")),
            ..Explorer::default()
        };
        app.restore_preferences(false);
        app.scan_build(root.into());
        finish(&mut app);
        app.open(root.join("app.elf"));
        finish(&mut app);
        app
    }
    fn fixture(root: &std::path::Path) {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(
            root.join("app.elf"),
            include_bytes!("../../../fixtures/build/cortex-m.elf"),
        )
        .unwrap();
        std::fs::write(root.join("app.su"), "diag.c:22:36:diagnose\t24\tstatic\n").unwrap();
    }
    #[test]
    fn snapshot_paths_fit_the_windows_path_budget_with_long_display_names() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("build");
        fixture(&root);
        let mut app = open(&root);
        app.take_snapshot(&"baseline".repeat(20)).unwrap();
        let mut snapshot = app.snapshots.snapshots[0].clone();
        snapshot.firmware = format!("nested/{}.elf", "firmware".repeat(20));
        // An ordinary Windows configuration directory. Its platform-independent
        // length plus the generated relative path must stay below MAX_PATH.
        let config = "C:\\Users\\Christopher\\AppData\\Roaming\\snout-firmware-explorer";
        let preferences = directory.path().join("workspace.json");
        let path = snapshot_path(&preferences, &root, &snapshot).unwrap();
        let relative = path.strip_prefix(directory.path()).unwrap();
        assert!(config.len() + 1 + relative.as_os_str().len() < 260);
        assert_eq!(path.file_stem().unwrap().len(), 32);
        snapshot.firmware = format!("other/{}.elf", "firmware".repeat(20));
        assert_ne!(snapshot_path(&preferences, &root, &snapshot).unwrap(), path);
    }
    #[test]
    fn snapshots_save_overwrite_reload_and_delete_with_paths_longer_than_max_path() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory
            .path()
            .join("long-config-directory-".repeat(5))
            .join("nested-config-directory-".repeat(5));
        let root = directory.path().join("build");
        fixture(&root);
        let preferences = config.join("workspace.json");
        let mut app = open(&root);
        app.preferences_file = Some(preferences.clone());
        let name = "Before update: CON / firmware? 🐽";
        app.take_snapshot(name).unwrap();
        let mut snapshot = app.snapshots.snapshots[0].clone();
        let path = snapshot_path(&preferences, &root, &snapshot).unwrap();
        assert!(path.as_os_str().len() > 260);
        assert!(path.is_file());
        assert!(write_snapshot(&preferences, &root, &snapshot, false).is_err());
        snapshot.analysis.totals.ram += 1024;
        write_snapshot(&preferences, &root, &snapshot, true).unwrap();
        app.load_snapshots();
        assert!(app.snapshot_error.is_none(), "{:?}", app.snapshot_error);
        assert_eq!(app.snapshots.snapshots.len(), 1);
        assert_eq!(app.snapshots.snapshots[0].name, name);
        assert_eq!(
            app.snapshots.snapshots[0].analysis.totals,
            snapshot.analysis.totals
        );
        app.delete_snapshot(name).unwrap();
        assert!(!path.exists());
        app.load_snapshots();
        assert!(app.snapshots.snapshots.is_empty());
    }
    #[test]
    fn legacy_persisted_baseline_selection_is_ignored() {
        let store: SnapshotStore = serde_json::from_value(serde_json::json!({
            "active": {"app.elf": "baseline"}
        }))
        .unwrap();
        assert!(store.active.is_empty());
        let mut store = store;
        store.active.insert("app.elf".into(), "baseline".into());
        assert!(serde_json::to_value(store).unwrap().get("active").is_none());
    }
    #[test]
    fn signed_deltas_preserve_unchanged_values_and_do_not_overflow() {
        assert_eq!(
            annotate("170.00 KiB".into(), 170 * 1024, Some(169 * 1024), false),
            "170.00 KiB (+1.00 KiB)"
        );
        assert_eq!(
            annotate("168.00 KiB".into(), 168 * 1024, Some(169 * 1024), false),
            "168.00 KiB (-1.00 KiB)"
        );
        assert_eq!(annotate("0 B".into(), 0, Some(0), false), "0 B");
        assert_eq!(
            annotate("0x20000400".into(), 0x20000400, Some(0x20000000), true),
            "0x20000400 (+0x400)"
        );
        assert_eq!(
            annotate("0x20000000".into(), 0x20000000, Some(0x20000400), true),
            "0x20000000 (-0x400)"
        );
        assert_eq!(
            annotate("0x0".into(), 0, Some(u64::MAX), true),
            "0x0 (-0xffffffffffffffff)"
        );
    }
    #[test]
    fn snapshots_survive_restart_but_baseline_selection_is_session_only() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("build");
        fixture(&root);
        let mut app = open(&root);
        app.select_stack_reports(vec![root.join("app.su")]);
        finish(&mut app);
        let original = app.analysis.as_ref().unwrap().totals;
        app.take_snapshot(" before rebuild ").unwrap();
        assert!(app.take_snapshot("before rebuild").is_err());
        assert!(app.take_snapshot("  ").is_err());
        assert!(app.snapshot_label().is_none());
        app.select_snapshot(Some("before rebuild".into())).unwrap();
        assert!(app.comparison.is_some());
        app.take_snapshot("second baseline").unwrap();
        std::fs::write(
            root.join("app.elf"),
            include_bytes!("../../../fixtures/build/cortex-m-grown.elf"),
        )
        .unwrap();
        std::fs::write(root.join("app.su"), "diag.c:99:36:diagnose\t96\tstatic\n").unwrap();
        app.refresh();
        finish(&mut app);
        let current = app.analysis.as_ref().unwrap().totals;
        assert_ne!(current, original);
        assert_eq!(app.snapshot_old("totals", "", "ram"), Some(original.ram));
        assert!(app
            .snapshot_bytes("totals", "", "ram", current.ram)
            .contains("(+"));
        let entry = &app.stack.as_ref().unwrap().entries[0];
        assert_eq!(
            app.snapshot_bytes("stack", &stack_key(entry), "local_bytes", entry.local_bytes),
            "96 B (+72 B)"
        );
        assert_eq!(
            app.comparison.as_ref().unwrap().ram_delta,
            i128::from(current.ram) - i128::from(original.ram)
        );
        let saved = std::fs::read(root.parent().unwrap().join("workspace.json")).unwrap();
        let workspace: serde_json::Value = serde_json::from_slice(&saved).unwrap();
        let settings = &workspace["build_settings"][root.to_str().unwrap()]["snapshots"];
        assert!(
            settings.get("snapshots").is_none(),
            "Snapshot reports must not be embedded in workspace.json"
        );
        assert!(
            settings.get("active").is_none(),
            "Baseline selection must not be saved"
        );
        let snapshot_files: Vec<_> = std::fs::read_dir(
            build_directory(app.preferences_file.as_ref().unwrap(), &root).unwrap(),
        )
        .unwrap()
        .flat_map(|entry| std::fs::read_dir(entry.unwrap().path()).unwrap())
        .map(|entry| entry.unwrap().path())
        .collect();
        assert_eq!(snapshot_files.len(), 2);
        for path in &snapshot_files {
            let stored: SnapshotFile =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            assert_eq!(stored.version, 1);
            assert_eq!(stored.build_folder, root);
            assert_eq!(stored.snapshot.analysis.totals, original);
        }
        assert_eq!(
            std::fs::read_dir(&root).unwrap().count(),
            2,
            "Taking a snapshot must not create any files in the build folder"
        );
        app.reset_build_settings();
        finish(&mut app);
        assert_eq!(app.snapshot_label(), Some("before rebuild"));
        let mut restarted = open(&root);
        assert!(restarted.snapshot_label().is_none());
        assert!(restarted.comparison.is_none());
        restarted
            .select_snapshot(Some("before rebuild".into()))
            .unwrap();
        assert_eq!(restarted.snapshot_analysis().unwrap().totals, original);
        assert_eq!(restarted.comparison.as_ref().unwrap().old, original);
        assert_eq!(
            restarted.snapshot_old("totals", "", "ram"),
            Some(original.ram)
        );
        std::fs::copy(root.join("app.elf"), root.join("other.elf")).unwrap();
        restarted.scan_build(root.clone());
        finish(&mut restarted);
        restarted.open(root.join("other.elf"));
        finish(&mut restarted);
        assert!(restarted.snapshot_label().is_none());
        assert!(restarted.comparison.is_none());
        restarted.open(root.join("app.elf"));
        finish(&mut restarted);
        assert_eq!(restarted.snapshot_label(), Some("before rebuild"));
        restarted.select_snapshot(None).unwrap();
        assert!(restarted.snapshot_label().is_none());
        assert!(restarted.comparison.is_none());
        assert!(!restarted
            .snapshot_bytes("totals", "", "ram", current.ram)
            .contains('('));
        assert!(open(&root).snapshot_label().is_none());
        assert_eq!(open(&root).snapshots.snapshots.len(), 2);
    }
    #[test]
    fn moved_symbols_match_by_identity_and_duplicate_baseline_symbols_are_unknown() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("build");
        fixture(&root);
        let mut app = open(&root);
        let symbol = app
            .analysis
            .as_ref()
            .unwrap()
            .symbols
            .iter()
            .find(|s| s.size > 0)
            .unwrap()
            .clone();
        app.take_snapshot("original").unwrap();
        app.select_snapshot(Some("original".into())).unwrap();
        let mut moved = symbol.clone();
        moved.address += 1024;
        moved.normalized_address += 1024;
        moved.size += 16;
        moved.source_line = Some(999);
        assert_eq!(symbol_key(&symbol), symbol_key(&moved));
        assert_eq!(
            app.snapshot_address("symbol", &symbol_key(&moved), "address", moved.address),
            format!("{:#010x} (+0x400)", moved.address)
        );
        assert_eq!(
            app.snapshot_bytes("symbol", &symbol_key(&moved), "size", moved.size),
            format!("{} (+16 B)", format_bytes(moved.size))
        );
        moved.name = "new_symbol".into();
        assert!(app
            .snapshot_bytes("symbol", &symbol_key(&moved), "size", moved.size)
            .ends_with("(new)"));
        let mut analysis = (**app.analysis.as_ref().unwrap()).clone();
        analysis.symbols.push(symbol.clone());
        app.analysis = Some(Arc::new(analysis));
        app.take_snapshot("ambiguous").unwrap();
        app.select_snapshot(Some("ambiguous".into())).unwrap();
        assert!(app
            .snapshot_bytes("symbol", &symbol_key(&symbol), "size", symbol.size)
            .ends_with("(baseline ambiguous)"));
    }
    #[test]
    fn duplicate_section_tls_range_and_stack_identities_do_not_produce_false_deltas() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("build");
        fixture(&root);
        let mut app = open(&root);
        app.select_stack_reports(vec![root.join("app.su")]);
        finish(&mut app);
        let mut analysis = (**app.analysis.as_ref().unwrap()).clone();
        let mut section = analysis.sections[0].clone();
        section.size += 7;
        let section_name = section.name.clone();
        analysis.sections.push(section);
        let mut range = analysis.memory_map[0].clone();
        let range_id = serde_json::to_string(&(&range.name, &range.space)).unwrap();
        range.size += 7;
        analysis.memory_map.push(range);
        analysis.tls = Some(firmware_analysis_core::TlsReport {
            source: "test".into(),
            initialized_size: 12,
            zero_initialized_size: 0,
            template_size: 12,
            alignment: 4,
            total_runtime_ram: None,
            symbols: vec![
                firmware_analysis_core::TlsSymbol {
                    name: "local_tls".into(),
                    offset: 0,
                    size: 4,
                    section: ".tdata".into(),
                },
                firmware_analysis_core::TlsSymbol {
                    name: "local_tls".into(),
                    offset: 4,
                    size: 8,
                    section: ".tdata".into(),
                },
            ],
        });
        app.analysis = Some(Arc::new(analysis));
        let stack = app.stack.as_mut().unwrap();
        let mut entry = stack.entries[0].clone();
        let stack_id = stack_key(&entry);
        entry.source_line += 1;
        entry.local_bytes += 7;
        stack.entries.push(entry);
        app.take_snapshot("duplicates").unwrap();
        app.select_snapshot(Some("duplicates".into())).unwrap();
        let identities = [
            ("section", section_name.as_str(), "size"),
            ("range", range_id.as_str(), "size"),
            ("tls_symbol", "local_tls", "size"),
            ("stack", stack_id.as_str(), "local_bytes"),
        ];
        for (domain, id, field) in identities {
            assert_eq!(app.snapshot_old(domain, id, field), None);
            assert_eq!(
                app.snapshot_bytes(domain, id, field, 4),
                "4 B (baseline ambiguous)"
            );
            assert!(app
                .snapshot_address(domain, id, field, 4)
                .ends_with("(baseline ambiguous)"));
            assert_eq!(
                app.snapshot_count(domain, id, field, 4),
                "4 (baseline ambiguous)"
            );
        }
        // Old snapshot files can contain last-row-wins derived values. Loading
        // must rebuild them from the captured report, preserving ambiguity.
        let mut saved = app.snapshots.snapshots[0].clone();
        for (domain, id, field) in identities {
            saved.values.remove(&key(domain, id, "ambiguous"));
            saved.values.insert(key(domain, id, field), 99);
        }
        write_snapshot(app.preferences_file.as_ref().unwrap(), &root, &saved, true).unwrap();
        let mut restarted = open(&root);
        restarted
            .select_snapshot(Some("duplicates".into()))
            .unwrap();
        for (domain, id, field) in identities {
            assert_eq!(restarted.snapshot_old(domain, id, field), None);
            assert_eq!(
                restarted.snapshot_bytes(domain, id, field, 4),
                "4 B (baseline ambiguous)"
            );
        }
    }

    #[test]
    fn duplicate_region_names_do_not_compare_against_the_last_region() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("build");
        fixture(&root);
        let mut app = open(&root);
        let mut analysis = (**app.analysis.as_ref().unwrap()).clone();
        let symbol = analysis
            .symbols
            .iter()
            .find(|s| s.size > 0)
            .unwrap()
            .clone();
        let region = firmware_analysis_core::MemoryRegion {
            name: "shared".into(),
            start: symbol.normalized_address,
            size: symbol.size,
            kind: firmware_analysis_core::MemoryKind::Flash,
        };
        analysis.options.regions = vec![
            region.clone(),
            firmware_analysis_core::MemoryRegion {
                start: region.start + region.size + 1024,
                size: region.size + 7,
                ..region.clone()
            },
        ];
        firmware_analysis_core::validate_options(&analysis.options).unwrap();
        app.analysis = Some(Arc::new(analysis));
        app.take_snapshot("duplicate regions").unwrap();
        app.select_snapshot(Some("duplicate regions".into()))
            .unwrap();
        assert_eq!(app.snapshot_old("region", "shared", "size"), None);
        assert_eq!(
            app.snapshot_bytes("region", "shared", "size", region.size),
            format!("{} (baseline ambiguous)", format_bytes(region.size))
        );
        assert_eq!(
            app.snapshot_region_percentage("shared", region.size, region.size),
            "100.0%"
        );
        let usage =
            firmware_analysis_core::regions::region_usage(app.analysis.as_ref().unwrap(), &region);
        let entry = usage
            .symbols
            .iter()
            .find(|e| {
                symbol_key(&app.analysis.as_ref().unwrap().symbols[e.symbol_index])
                    == symbol_key(&symbol)
            })
            .unwrap();
        assert!(app
            .snapshot_placement_address(&symbol, "shared", entry.placement, entry.address)
            .ends_with("(baseline ambiguous)"));
        // Ambiguous placement must not hide the symbol's unambiguous size.
        assert_eq!(
            app.snapshot_old("symbol", &symbol_key(&symbol), "size"),
            Some(symbol.size)
        );
        let mut restarted = open(&root);
        restarted
            .select_snapshot(Some("duplicate regions".into()))
            .unwrap();
        assert_eq!(restarted.snapshot_old("region", "shared", "used"), None);
    }

    #[test]
    fn failed_snapshot_save_does_not_change_baseline_or_snapshot_list() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("build");
        fixture(&root);
        let mut app = open(&root);
        app.take_snapshot("original").unwrap();
        app.select_snapshot(Some("original".into())).unwrap();
        let mut unsaved = app.snapshots.snapshots[0].clone();
        unsaved.name = "unsaved".into();
        let blocked =
            snapshot_path(app.preferences_file.as_ref().unwrap(), &root, &unsaved).unwrap();
        std::fs::create_dir(&blocked).unwrap();
        assert!(app.take_snapshot("unsaved").is_err());
        let path = app.preferences_file.clone().unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(path).unwrap();
        assert_eq!(app.snapshot_label(), Some("original"));
        app.select_snapshot(None).unwrap();
        assert!(app.snapshot_label().is_none());
        assert!(app.comparison.is_none());
        assert_eq!(app.snapshots.snapshots.len(), 1);
    }
    #[test]
    fn snapshot_files_are_isolated_by_build_firmware_and_name_and_corruption_is_local() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("build");
        let other = directory.path().join("other-build");
        fixture(&root);
        fixture(&other);
        let mut app = open(&root);
        app.take_snapshot("a/b").unwrap();
        app.take_snapshot("a?b").unwrap();
        let preferences = app.preferences_file.clone().unwrap();
        let first = snapshot_path(&preferences, &root, &app.snapshots.snapshots[0]).unwrap();
        let second = snapshot_path(&preferences, &root, &app.snapshots.snapshots[1]).unwrap();
        assert_ne!(first, second);
        assert!(first.starts_with(directory.path().join("snapshots")));
        let saved = std::fs::read(&first).unwrap();
        app.select_snapshot(Some("a/b".into())).unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), saved);
        let mut other_app = open(&other);
        assert!(other_app.snapshots.snapshots.is_empty());
        other_app.take_snapshot("a/b").unwrap();
        let other_path =
            snapshot_path(&preferences, &other, &other_app.snapshots.snapshots[0]).unwrap();
        assert_ne!(first, other_path);
        assert!(other_path.is_file());
        std::fs::write(&second, b"broken json").unwrap();
        let mut restarted = open(&root);
        assert!(restarted.snapshot_error.is_some());
        assert_eq!(restarted.snapshots.snapshots.len(), 1);
        assert_eq!(restarted.snapshots.snapshots[0].name, "a/b");
        assert!(restarted.take_snapshot("a?b").is_err());
        assert_eq!(std::fs::read(&second).unwrap(), b"broken json");
        assert_eq!(std::fs::read(&first).unwrap(), saved);
    }
    #[test]
    fn startup_removes_orphan_snapshot_folders_and_keeps_registered_missing_builds() {
        let directory = tempfile::tempdir().unwrap();
        let preferences = directory.path().join("workspace.json");
        let registered = directory.path().join("registered-but-missing-build");
        let orphan = directory.path().join("removed-build");
        let kept = build_directory(&preferences, &registered).unwrap();
        let removed = build_directory(&preferences, &orphan).unwrap();
        for folder in [&kept, &removed] {
            std::fs::create_dir_all(folder.join("app.elf")).unwrap();
            std::fs::write(folder.join("app.elf/baseline.json"), b"snapshot contents").unwrap();
        }
        let unrelated = directory.path().join("outside.json");
        std::fs::write(&unrelated, b"keep").unwrap();
        let settings =
            BTreeMap::from([(registered.clone(), super::super::BuildSettings::default())]);
        let value = serde_json::json!({"version": 1, "build_settings": settings});
        std::fs::write(&preferences, serde_json::to_vec(&value).unwrap()).unwrap();
        let mut app = Explorer {
            preferences_file: Some(preferences.clone()),
            ..Default::default()
        };
        app.restore_preferences(false);
        assert!(kept.join("app.elf/baseline.json").is_file());
        assert!(!removed.exists());
        assert!(unrelated.is_file());
        assert!(!registered.exists());
        assert!(app.build_settings.contains_key(&registered));
        // Once the final build-settings entry is removed, its snapshots are orphaned too.
        std::fs::write(&preferences, b"{\"version\":1,\"build_settings\":{}}").unwrap();
        app.restore_preferences(false);
        assert!(!kept.exists());
        assert!(unrelated.is_file());
    }
    #[test]
    fn startup_cleanup_skips_missing_corrupt_or_unsupported_build_settings() {
        let directory = tempfile::tempdir().unwrap();
        let preferences = directory.path().join("workspace.json");
        let orphan = build_directory(&preferences, &directory.path().join("old-build")).unwrap();
        std::fs::create_dir_all(&orphan).unwrap();
        let snapshot = orphan.join("keep.json");
        std::fs::write(&snapshot, b"keep").unwrap();
        let mut app = Explorer {
            preferences_file: Some(preferences.clone()),
            ..Default::default()
        };
        app.restore_preferences(false);
        assert!(snapshot.is_file());
        for data in [
            "broken json",
            "{\"version\":2,\"build_settings\":{}}",
            "{\"version\":1}",
            "{\"version\":1,\"build_settings\":null}",
            "{\"version\":1,\"build_settings\":{\"build\":false}}",
        ] {
            std::fs::write(&preferences, data).unwrap();
            app.restore_preferences(false);
            assert!(
                snapshot.is_file(),
                "Cleanup must skip invalid preferences: {data}"
            );
        }
    }
    #[cfg(unix)]
    #[test]
    fn startup_cleanup_does_not_follow_symlinks_outside_snapshot_storage() {
        let directory = tempfile::tempdir().unwrap();
        let preferences = directory.path().join("workspace.json");
        let outside = directory.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep.json"), b"keep").unwrap();
        let snapshots = directory.path().join("snapshots");
        std::fs::create_dir(&snapshots).unwrap();
        std::os::unix::fs::symlink(&outside, snapshots.join("orphan-link")).unwrap();
        let orphan = snapshots.join("orphan-build");
        std::fs::create_dir(&orphan).unwrap();
        std::os::unix::fs::symlink(&outside, orphan.join("firmware-link")).unwrap();
        std::fs::write(&preferences, b"{\"version\":1,\"build_settings\":{}}").unwrap();
        let mut app = Explorer {
            preferences_file: Some(preferences),
            ..Default::default()
        };
        app.restore_preferences(false);
        assert!(outside.join("keep.json").is_file());
        assert!(snapshots.join("orphan-link").is_symlink());
        assert!(!orphan.exists());
    }
    #[test]
    fn snapshot_manager_stays_stationary_at_fractional_display_scales() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("build");
        fixture(&root);
        let mut app = open(&root);
        app.take_snapshot("Before adding Bluetooth and enabling the diagnostics subsystem")
            .unwrap();
        let saved = app.snapshots.snapshots[0].clone();
        for count in [0, 1, 3] {
            app.snapshots.snapshots = vec![saved.clone(); count];
            for (index, snapshot) in app.snapshots.snapshots.iter_mut().enumerate().skip(1) {
                snapshot.name = format!("{} {index}", saved.name);
            }
            if count > 0 {
                app.select_snapshot(Some(saved.name.clone())).unwrap();
            }
            for scale in [1.0, 1.1, 1.25, 1.5, 1.75, 2.0] {
                app.open_snapshot_manager();
                let ctx = egui::Context::default();
                super::super::shell::configure_style(&ctx);
                ctx.set_pixels_per_point(scale);
                for physical_height in [500, 570, 577, 601, 773, 801, 843] {
                    let screen = egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(801.0 / scale, physical_height as f32 / scale),
                    );
                    let mut settled = None;
                    for frame in 0..20 {
                        let _ = ctx.run(
                            egui::RawInput {
                                screen_rect: Some(screen),
                                ..Default::default()
                            },
                            |ctx| app.show_snapshot_dialog(ctx),
                        );
                        let rect = ctx
                            .memory(|memory| memory.area_rect(egui::Id::new("snapshot_manager")))
                            .unwrap();
                        if frame >= 10 {
                            if let Some(previous) = settled {
                                assert_eq!(rect, previous, "Snapshot manager moved at scale {scale}, count {count}, height {physical_height}, frame {frame}");
                            }
                            settled = Some(rect);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn snapshot_manager_is_compact_centered_and_places_the_name_label_left_of_the_field() {
        for size in [egui::vec2(800.0, 600.0), egui::vec2(1280.0, 900.0)] {
            let mut app = Explorer::default();
            app.open_snapshot_manager();
            let ctx = egui::Context::default();
            super::super::shell::configure_style(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            let mut output = None;
            for _ in 0..3 {
                output = Some(ctx.run(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        ..Default::default()
                    },
                    |ctx| app.show_snapshot_dialog(ctx),
                ));
            }
            let rect = ctx
                .memory(|memory| memory.area_rect(egui::Id::new("snapshot_manager")))
                .unwrap();
            assert!(
                rect.height() < 400.0,
                "Manager has excess blank space: {rect:?}"
            );
            assert!(
                (rect.center() - screen.center()).length() < 2.0,
                "Manager is not centered: {rect:?}"
            );
            let output = output.unwrap();
            let text_rect = |label: &str| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => {
                            Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                        }
                        _ => None,
                    })
                    .unwrap()
            };
            let close = text_rect("Close");
            assert!(rect.bottom() - close.bottom() < 40.0);
            let label = text_rect("Snapshot name:");
            let hint = text_rect("e.g. Before adding Bluetooth");
            assert!(label.right() < hint.left());
            assert!((label.center().y - hint.center().y).abs() < 2.0);
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Rect(rect) if rect.fill.r() == 0 && rect.fill.a() > 0 && rect.rect.width() >= screen.width()-1.0 && rect.rect.height() >= screen.height()-1.0)));
        }
    }
    fn finish_save(app: &mut Explorer) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while app.snapshot_job.is_some() {
            app.snapshot_job.as_mut().unwrap().presented = true;
            app.poll_snapshot_save();
            assert!(
                std::time::Instant::now() < deadline,
                "Snapshot save timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn enter_in_snapshot_name_field_saves_a_named_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("build");
        fixture(&root);
        let mut app = open(&root);
        app.open_snapshot_manager();
        let ctx = egui::Context::default();
        let frame = |app: &mut Explorer, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| app.show_snapshot_dialog(ctx),
            )
        };
        let _ = frame(&mut app, vec![]);
        let _ = frame(&mut app, vec![egui::Event::Text("Before update".into())]);
        assert_eq!(app.snapshot_name, "Before update");
        let _ = frame(
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(matches!(app.snapshot_dialog, Some(Dialog::Saving(_))));
        let output = frame(&mut app, vec![]);
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(text) if text.galley.text().contains("Saving snapshot"))));
        finish_save(&mut app);
        assert!(matches!(app.snapshot_dialog, Some(Dialog::Manager)));
        assert_eq!(app.snapshots.snapshots.len(), 1);
        assert_eq!(app.snapshots.snapshots[0].name, "Before update");
        assert!(snapshot_path(
            app.preferences_file.as_ref().unwrap(),
            &root,
            &app.snapshots.snapshots[0]
        )
        .unwrap()
        .is_file());
    }
    fn manager_frame(
        ctx: &egui::Context,
        app: &mut Explorer,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.show_snapshot_dialog(ctx),
        )
    }
    fn click_manager(ctx: &egui::Context, app: &mut Explorer, label: &str) {
        let mut output = manager_frame(ctx, app, vec![]);
        for _ in 0..2 {
            output = manager_frame(ctx, app, vec![]);
        }
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() / 2.0)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing manager action {label}"));
        click_manager_at(ctx, app, pos);
    }
    fn click_manager_bin(ctx: &egui::Context, app: &mut Explorer) {
        let mut output = manager_frame(ctx, app, vec![]);
        for _ in 0..2 {
            output = manager_frame(ctx, app, vec![]);
        }
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.rect.size() == egui::vec2(10.0, 12.0) => {
                    Some(rect.rect.center())
                }
                _ => None,
            })
            .expect("Missing snapshot bin icon");
        click_manager_at(ctx, app, pos);
    }
    fn click_manager_at(ctx: &egui::Context, app: &mut Explorer, pos: egui::Pos2) {
        for pressed in [true, false] {
            let _ = manager_frame(
                ctx,
                app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }
    #[test]
    fn duplicate_names_ask_before_overwriting_and_replace_the_active_baseline_after_confirmation() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("build");
        fixture(&root);
        let mut app = open(&root);
        app.take_snapshot("baseline").unwrap();
        app.select_snapshot(Some("baseline".into())).unwrap();
        let path = snapshot_path(
            app.preferences_file.as_ref().unwrap(),
            &root,
            &app.snapshots.snapshots[0],
        )
        .unwrap();
        let original = std::fs::read(&path).unwrap();
        let old_ram = app.snapshot_old("totals", "", "ram").unwrap();
        let mut analysis = (**app.analysis.as_ref().unwrap()).clone();
        analysis.totals.ram += 1024;
        app.analysis = Some(Arc::new(analysis));
        app.open_snapshot_manager();
        app.snapshot_name = " baseline ".into();
        let ctx = egui::Context::default();
        app.request_snapshot_save(&ctx).unwrap();
        assert!(matches!(&app.snapshot_dialog,Some(Dialog::Overwrite(name)) if name == "baseline"));
        assert!(app.snapshot_job.is_none());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        click_manager(&ctx, &mut app, "Cancel");
        assert!(matches!(app.snapshot_dialog, Some(Dialog::Manager)));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        app.request_snapshot_save(&ctx).unwrap();
        click_manager(&ctx, &mut app, "Overwrite");
        assert!(matches!(app.snapshot_dialog, Some(Dialog::Saving(_))));
        assert_eq!(app.snapshot_old("totals", "", "ram"), Some(old_ram));
        let output = manager_frame(&ctx, &mut app, vec![]);
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(text) if text.galley.text().contains("Saving snapshot"))));
        finish_save(&mut app);
        assert_eq!(app.snapshots.snapshots.len(), 1);
        assert_eq!(app.snapshot_old("totals", "", "ram"), Some(old_ram + 1024));
        assert_eq!(app.comparison.as_ref().unwrap().ram_delta, 0);
        let mut restarted = open(&root);
        assert!(restarted.snapshot_label().is_none());
        restarted.select_snapshot(Some("baseline".into())).unwrap();
        assert_eq!(
            restarted.snapshot_old("totals", "", "ram"),
            Some(old_ram + 1024)
        );
    }
    #[test]
    fn manager_compares_deletes_and_clears_the_deleted_baseline_across_restart() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("build");
        fixture(&root);
        let mut app = open(&root);
        app.take_snapshot("baseline").unwrap();
        let path = snapshot_path(
            app.preferences_file.as_ref().unwrap(),
            &root,
            &app.snapshots.snapshots[0],
        )
        .unwrap();
        app.open_snapshot_manager();
        let ctx = egui::Context::default();
        click_manager_bin(&ctx, &mut app);
        assert!(matches!(app.snapshot_dialog, Some(Dialog::Delete(_))));
        assert!(
            app.snapshot_label().is_none(),
            "The bin must not start a comparison"
        );
        click_manager(&ctx, &mut app, "Cancel");
        click_manager(&ctx, &mut app, "baseline");
        assert_eq!(app.snapshot_label(), Some("baseline"));
        assert!(app.snapshot_dialog.is_none());
        app.open_snapshot_manager();
        click_manager(&ctx, &mut app, "Stop comparing");
        assert!(app.snapshot_label().is_none());
        assert!(path.is_file());
        click_manager(&ctx, &mut app, "ELF: app.elf");
        assert_eq!(app.snapshot_label(), Some("baseline"));
        assert!(
            app.snapshot_dialog.is_none(),
            "Snapshot details must also select the baseline"
        );
        app.open_snapshot_manager();
        click_manager_bin(&ctx, &mut app);
        assert!(matches!(app.snapshot_dialog, Some(Dialog::Delete(_))));
        click_manager(&ctx, &mut app, "Cancel");
        assert!(path.is_file());
        assert_eq!(app.snapshot_label(), Some("baseline"));
        click_manager_bin(&ctx, &mut app);
        click_manager(&ctx, &mut app, "Delete snapshot");
        assert!(!path.exists());
        assert!(app.snapshots.snapshots.is_empty());
        assert!(app.snapshot_label().is_none());
        let restarted = open(&root);
        assert!(restarted.snapshots.snapshots.is_empty());
        assert!(restarted.snapshot_label().is_none());
        assert!(restarted.comparison.is_none());
    }
    #[test]
    fn failed_background_saves_show_errors_inside_the_snapshot_manager() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("build");
        fixture(&root);
        let mut app = open(&root);
        app.take_snapshot("baseline").unwrap();
        let mut blocked = app.snapshots.snapshots[0].clone();
        blocked.name = "blocked".into();
        let path = snapshot_path(app.preferences_file.as_ref().unwrap(), &root, &blocked).unwrap();
        std::fs::create_dir(&path).unwrap();
        app.open_snapshot_manager();
        app.snapshot_name = "blocked".into();
        let ctx = egui::Context::default();
        app.request_snapshot_save(&ctx).unwrap();
        assert!(matches!(app.snapshot_dialog, Some(Dialog::Overwrite(_))));
        click_manager(&ctx, &mut app, "Overwrite");
        finish_save(&mut app);
        assert!(app.snapshot_dialog_error.is_some());
        assert_eq!(app.snapshots.snapshots.len(), 1);
        let error = app.snapshot_dialog_error.clone().unwrap();
        let output = manager_frame(&ctx, &mut app, vec![]);
        assert!(output.shapes.iter().any(
            |shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.text() == error)
        ));
    }
    #[test]
    fn snapshot_modal_blocks_background_shortcuts_and_dragged_firmware() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("build");
        fixture(&root);
        let mut app = open(&root);
        app.open_snapshot_manager();
        let ctx = egui::Context::default();
        let revision = app.report_revision;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                events: vec![egui::Event::Key {
                    key: egui::Key::F5,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                dropped_files: vec![egui::DroppedFile {
                    path: Some(root.join("app.elf")),
                    ..Default::default()
                }],
                ..Default::default()
            },
            |ctx| app.show(ctx),
        );
        assert!(app.receiver.is_none());
        assert_eq!(app.report_revision, revision);
        assert!(app.snapshot_dialog.is_some());
    }
    #[test]
    fn snapshot_capture_time_and_elf_are_persisted_and_visible_in_the_manager() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("build");
        fixture(&root);
        let mut app = open(&root);
        app.take_snapshot("baseline").unwrap();
        let time = app.snapshots.snapshots[0].taken_at;
        let mut restarted = open(&root);
        assert_eq!(restarted.snapshots.snapshots[0].taken_at, time);
        assert_eq!(restarted.snapshots.snapshots[0].firmware, "app.elf");
        let label = restarted.snapshots.snapshots[0].time_label();
        assert!(label.starts_with("Taken: "));
        restarted.open_snapshot_manager();
        let ctx = egui::Context::default();
        let _ = manager_frame(&ctx, &mut restarted, vec![]);
        let output = manager_frame(&ctx, &mut restarted, vec![]);
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        assert!(texts.iter().any(|text| text == "ELF: app.elf"), "{texts:?}");
        assert!(texts.iter().any(|text| text == &label), "{texts:?}");
        for shape in &output.shapes {
            if let egui::Shape::Text(text) = &shape.shape {
                if text.galley.text() == "ELF: app.elf" || text.galley.text() == label {
                    assert!(
                        text.galley
                            .job
                            .sections
                            .iter()
                            .all(|section| section.format.font_id.size >= 14.0),
                        "Snapshot metadata must remain readable"
                    );
                }
            }
        }
    }
    #[test]
    fn default_tabs_and_compare_share_the_snapshot_baseline() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("build");
        fixture(&root);
        let mut app = open(&root);
        app.take_snapshot("baseline").unwrap();
        app.select_snapshot(Some("baseline".into())).unwrap();
        let mut analysis = (**app.analysis.as_ref().unwrap()).clone();
        let symbol = analysis.symbols.iter_mut().find(|s| s.size > 0).unwrap();
        symbol.address += 1024;
        symbol.size += 16;
        let name = symbol.demangled_name.clone();
        analysis.sections[0].size += 1024;
        let a = Arc::new(analysis);
        app.analysis = Some(a.clone());
        app.sync_snapshot_comparison();
        app.report_revision += 1;
        app.change_view(super::super::View::Symbols);
        app.search = name;
        let ctx = egui::Context::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 900.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.symbols(ui, &a));
            },
        );
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        assert!(
            texts.iter().any(|text| text.contains("(+16 B)")),
            "{texts:?}"
        );
        assert!(
            texts.iter().any(|text| text.contains("(+0x400)")),
            "{texts:?}"
        );
        assert!(app.comparison.is_some());
        app.change_view(super::super::View::Compare);
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.compare_view(ui));
        });
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "Baseline: baseline")));
        app.select_snapshot(None).unwrap();
        assert!(app.comparison.is_none());
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.compare_view(ui));
        });
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "Select baseline...")));
        assert!(!app
            .snapshot_address("symbol", "missing", "address", 0)
            .contains('('));
    }
}
