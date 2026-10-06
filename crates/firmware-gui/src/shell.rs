use super::display::display_path;
use super::{egui, Explorer, View};
use firmware_analysis_core::format_bytes as bytes;

// egui 0.30's nested menus only open to the right. Keep a separate left-hand
// area and include it in the parent menu's hit test between frames.
fn menu_with_left_submenu(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    let bar_id = ui.id();
    let child_id = bar_id.with("recent_menu_rect");
    let mut state = egui::menu::BarState::load(ui.ctx(), bar_id);
    let button = ui.button("Menu");
    if let Some(root) = state.as_ref() {
        if let Some(child) = ui.ctx().data(|data| data.get_temp::<egui::Rect>(child_id)) {
            // Only extend the hit bounds when the pointer is actually in the
            // child. A permanent union also counts empty space beside either
            // menu as inside, preventing outside clicks from dismissing it.
            if ui.input(|input| {
                input
                    .pointer
                    .interact_pos()
                    .is_some_and(|pos| child.contains(pos))
            }) {
                let mut menu = root.menu_state.write();
                menu.rect = menu.rect.union(child);
            }
        }
    }
    egui::menu::MenuRoot::stationary_click_interaction(&button, &mut state);
    ui.ctx()
        .data_mut(|data| data.remove::<egui::Rect>(child_id));
    state.show(&button, contents);
    if state.as_ref().is_none() {
        ui.ctx().data_mut(|data| {
            data.insert_temp(bar_id.with("recent_menu"), false);
        });
    }
    state.store(ui.ctx(), bar_id);
}

fn left_recent_menu(
    ui: &mut egui::Ui,
    folders: &[std::path::PathBuf],
    bar_id: egui::Id,
) -> Option<std::path::PathBuf> {
    let id = bar_id.with("recent_menu");
    let button = ui.add_enabled(
        !folders.is_empty(),
        egui::Button::new("◀ Open recent build folder"),
    );
    let mut open = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(id).unwrap_or(false));
    if button.hovered() || button.clicked() {
        open = true;
    }
    let mut selected = None;
    if open && !folders.is_empty() {
        let parent = ui.max_rect();
        let margin = egui::Frame::menu(ui.style()).total_margin();
        let anchor = egui::pos2(
            parent.left() - margin.left - ui.spacing().menu_spacing,
            button.rect.top() - margin.top,
        );
        // Bound the popup to the space left of its parent. Otherwise long
        // paths make Area's screen constraint move it over the parent menu.
        let width = (anchor.x - ui.ctx().screen_rect().left() - margin.sum().x).clamp(1.0, 400.0);
        let popup = egui::Area::new(id)
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(anchor)
            .default_width(width + margin.sum().x)
            .sense(egui::Sense::hover())
            .show(ui.ctx(), |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    ui.set_width(width);
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                        for folder in folders {
                            let path = folder.to_string_lossy();
                            if ui
                                .add(egui::Button::new(display_path(&path)).truncate())
                                .on_hover_text(display_path(&path))
                                .clicked()
                            {
                                selected = Some(folder.clone());
                            }
                        }
                    });
                });
            });
        ui.ctx()
            .set_sublayer(ui.layer_id(), popup.response.layer_id);
        let hovering_other = ui.rect_contains_pointer(parent) && !button.hovered();
        if hovering_other
            || selected.is_some()
            || ui.input(|input| input.key_pressed(egui::Key::Escape))
        {
            open = false;
        }
        if open {
            ui.ctx().data_mut(|data| {
                data.insert_temp(bar_id.with("recent_menu_rect"), popup.response.rect)
            });
        }
    }
    ui.ctx().data_mut(|data| data.insert_temp(id, open));
    if selected.is_some() {
        ui.close_menu();
    }
    selected
}

pub(super) fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = egui::Color32::from_rgb(27, 30, 35);
    style.visuals.window_fill = egui::Color32::from_rgb(32, 36, 42);
    style.visuals.extreme_bg_color = egui::Color32::from_rgb(21, 24, 29);
    style.visuals.faint_bg_color = egui::Color32::from_white_alpha(4);
    style.visuals.override_text_color = Some(egui::Color32::from_rgb(205, 211, 220));
    style.visuals.selection.bg_fill = egui::Color32::from_rgb(39, 62, 79);
    style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, super::views::ACCENT);
    style.visuals.widgets.noninteractive.bg_stroke =
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(48, 53, 61));
    style.visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(36, 40, 47);
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    for widgets in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        widgets.rounding = egui::Rounding::same(3.0);
        widgets.expansion = 0.0;
    }
    style.spacing.item_spacing = egui::vec2(8.0, 4.0);
    style.spacing.button_padding = egui::vec2(7.0, 3.0);
    style.spacing.interact_size.y = 23.0;
    for text_style in [
        egui::TextStyle::Body,
        egui::TextStyle::Button,
        egui::TextStyle::Monospace,
    ] {
        if let Some(font) = style.text_styles.get_mut(&text_style) {
            font.size = 13.0;
        }
    }
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(17.0));
    ctx.set_style(style);
}

impl Explorer {
    pub(super) fn clear_region_filters(&mut self) {
        self.selected_region = None;
        for options in &mut self.tab_options {
            options.selected_region = None;
        }
    }

    pub(super) fn clear_firmware_filters(&mut self) {
        self.clear_region_filters();
        self.selected_file = None;
        for options in &mut self.tab_options {
            options.selected_file = None;
        }
    }

    pub(super) fn change_view(&mut self, view: View) {
        if self.view == view {
            return;
        }
        let index = View::ALL.iter().position(|v| *v == self.view).unwrap();
        self.tab_options[index] = super::TabOptions {
            search: std::mem::take(&mut self.search),
            sort_column: self.sort_column,
            descending: self.descending,
            tree: self.tree,
            selected_file: self.selected_file.take(),
            selected_region: self.selected_region.take(),
            kind_filter: std::mem::take(&mut self.kind_filter),
        };
        let index = View::ALL.iter().position(|v| *v == view).unwrap();
        let options = self.tab_options[index].clone();
        self.search = options.search;
        self.sort_column = options.sort_column;
        self.descending = options.descending;
        self.tree = options.tree;
        self.selected_file = options.selected_file;
        self.selected_region = options.selected_region;
        self.kind_filter = options.kind_filter;
        self.view = view;
        self.details = None;
        self.visible_rows = 0;
    }

    pub(super) fn show_file_symbols(&mut self, path: String) {
        self.change_view(View::Symbols);
        self.search.clear();
        self.details = None;
        self.selected_file = Some(path);
        self.kind_filter = "All".into();
    }

    pub(super) fn show(&mut self, ctx: &egui::Context) {
        let snapshot_modal_open = self.snapshot_dialog.is_some();
        self.show_snapshot_dialog(ctx);
        if !snapshot_modal_open {
            if let Some(path) =
                ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()))
            {
                if self.receiver.is_none() {
                    match super::startup::parse([path.into_os_string()]) {
                        Ok(Some(startup)) => self.open_startup(&startup),
                        Ok(None) => {}
                        Err(error) => self.error = Some(error),
                    }
                }
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::O))
                && self.receiver.is_none()
            {
                self.pick_build();
            }
            // Leave Escape available for egui to dismiss the menu hierarchy too.
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.details = None;
                self.show_notes = false;
                self.show_about = false;
            }
            if self.view == View::Overview
                && ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1))
            {
                self.overview_back();
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F5)) {
                self.refresh();
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F1))
                && self.receiver.is_none()
            {
                self.show_about = true;
            }
        }
        let title = self
            .build
            .as_ref()
            .map(|build| {
                let path = build.root.display().to_string();
                let path = display_path(&path);
                format!("{path} - Rusty's Snout - Firmware Explorer")
            })
            .unwrap_or_else(|| "Rusty's Snout - Firmware Explorer".into());
        if ctx.input(|i| i.viewport().title.as_deref() != Some(title.as_str())) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
        egui::TopBottomPanel::top("workbench_tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                for view in View::ALL {
                    let active = self.view == view;
                    let response = ui.add(
                        egui::Button::new(view.label())
                            .frame(false)
                            .min_size(egui::vec2(62.0, 28.0)),
                    );
                    if active {
                        let rect = response.rect;
                        ui.painter().line_segment(
                            [rect.left_bottom(), rect.right_bottom()],
                            egui::Stroke::new(2.0_f32, super::views::ACCENT),
                        );
                    }
                    if response.clicked() {
                        self.change_view(view);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(self.receiver.is_none(), |ui| {
                        let bar_id = ui.id();
                        menu_with_left_submenu(ui, |ui| {
                            if ui
                                .add(
                                    egui::Button::new("Open build folder...").shortcut_text(
                                        egui::RichText::new("Ctrl+O")
                                            .color(egui::Color32::from_gray(145)),
                                    ),
                                )
                                .clicked()
                            {
                                ui.close_menu();
                                self.pick_build();
                            }
                            if let Some(folder) =
                                left_recent_menu(ui, &self.recent_build_folders, bar_id)
                            {
                                self.scan_build(folder);
                            }
                            if ui
                                .add_enabled(
                                    self.build.is_some(),
                                    egui::Button::new("Refresh").shortcut_text(
                                        egui::RichText::new("F5")
                                            .color(egui::Color32::from_gray(145)),
                                    ),
                                )
                                .on_hover_text("Rescan and reload selected firmware")
                                .clicked()
                            {
                                ui.close_menu();
                                self.refresh();
                            }
                            if ui
                                .add_enabled(
                                    self.analysis.is_some() && self.receiver.is_none(),
                                    egui::Button::new("Snapshot"),
                                )
                                .clicked()
                            {
                                self.open_snapshot_manager();
                                ui.close_menu();
                            }
                            if ui
                                .add_enabled(
                                    self.build.is_some(),
                                    egui::Button::new("Reset settings for this build folder"),
                                )
                                .clicked()
                            {
                                ui.close_menu();
                                self.reset_build_settings();
                            }
                            ui.separator();
                            if ui
                                .add_enabled(
                                    self.updates.idle(),
                                    egui::Button::new("Check for updates"),
                                )
                                .on_disabled_hover_text(
                                    "An update check or installation is in progress",
                                )
                                .clicked()
                            {
                                self.start_update_check(ctx, true);
                                ui.close_menu();
                            }
                            if ui
                                .checkbox(
                                    &mut self.updates.check_on_startup,
                                    "Check for updates on startup",
                                )
                                .changed()
                            {
                                if let Err(error) = self.save_preferences() {
                                    self.error =
                                        Some(format!("Could not save update preferences: {error}"));
                                }
                            }
                            ui.separator();
                            if ui
                                .button("Support developer")
                                .on_hover_text("Opens Buy Me a Coffee in your browser")
                                .clicked()
                            {
                                ctx.open_url(egui::OpenUrl::new_tab(
                                    "https://buymeacoffee.com/rustypig91g",
                                ));
                                ui.close_menu();
                            }
                            if ui
                                .add(egui::Button::new("About").shortcut_text(
                                    egui::RichText::new("F1").color(egui::Color32::from_gray(145)),
                                ))
                                .clicked()
                            {
                                self.show_about = true;
                                ui.close_menu();
                            }
                        });
                    });
                });
            });
        });
        egui::TopBottomPanel::bottom("workbench_status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(a) = &self.analysis {
                    ui.small(format!(
                        "{} / {}-bit",
                        a.metadata.architecture, a.metadata.bitness
                    ))
                    .on_hover_text(format!(
                        "{} endian | Entry: {:#x}",
                        a.metadata.endianness, a.metadata.entry_point
                    ));
                    ui.separator();
                    ui.small(format!("Flash {}", bytes(a.totals.flash)));
                    ui.small(format!("RAM {}", bytes(a.totals.ram)))
                        .on_hover_text(
                            "Static RAM only; additional stack and heap demand may be unknown",
                        );
                    ui.separator();
                    ui.small(format!("{} rows", self.visible_rows));
                    if let Some(name) = self.snapshot_label() {
                        ui.separator();
                        ui.small(format!("Snapshot: {name}"))
                            .on_hover_text("Selected comparison baseline · current minus snapshot");
                    }
                } else {
                    ui.small("Ready / Select or drop a build folder or ELF to begin");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.analysis.is_some()
                        && ui
                            .selectable_label(
                                self.show_notes,
                                format!("{} notes", self.visible_notes().len()),
                            )
                            .clicked()
                    {
                        self.show_notes = !self.show_notes;
                    }
                    if self.receiver.is_some() {
                        ui.spinner();
                        ui.small("Analyzing...");
                    } else {
                        ui.small("Bytes / KiB").on_hover_text("1 KiB = 1,024 bytes");
                    }
                });
            });
        });
        if self.show_notes {
            egui::TopBottomPanel::bottom("analysis_notes")
                .resizable(true)
                .default_height(155.0)
                .min_height(65.0)
                .max_height(300.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong("Analysis notes");
                        if ui.small_button("Hide").clicked() {
                            self.show_notes = false;
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for note in self.visible_notes() {
                            ui.label(note);
                        }
                    });
                });
        }
        self.show_about_window(ctx);
        self.build_browser(ctx);
        if self.tree && matches!(self.view, View::Files | View::Symbols) {
            if let Some(a) = self.analysis.clone() {
                egui::SidePanel::left("file_tree")
                    .default_width(240.0)
                    .width_range(160.0..=420.0)
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.strong("DIRECTORIES");
                            if ui.small_button("Hide").clicked() {
                                self.tree = false;
                            }
                        });
                        ui.separator();
                        self.directory_tree(ui, &a);
                    });
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(error) = self.error.clone() {
                ui.horizontal_wrapped(|ui| { ui.colored_label(egui::Color32::LIGHT_RED, error); if ui.small_button("Dismiss").clicked() { self.error = None; } }); ui.separator();
            }
            if let Some(error) = self.snapshot_error.clone() {
                ui.horizontal_wrapped(|ui| { ui.colored_label(egui::Color32::LIGHT_RED, error); if ui.small_button("Dismiss snapshot error").clicked() { self.snapshot_error = None; } }); ui.separator();
            }
            if self.view != View::Overview && self.view != View::Compare {
                if let Some(name) = self.snapshot_label() { ui.label(format!("Comparing to snapshot: {name} · current minus baseline")); ui.separator(); }
            }
            if self.view == View::Overview && self.artifact_preview(ui) { return; }
            let Some(a) = self.analysis.clone() else {
                ui.add_space(24.0); ui.heading("Firmware Explorer");
                ui.label(if self.build.is_some() { "Select a firmware image or supporting file in the left pane." } else { "Select a build folder to discover firmware, maps and stack reports." });
                ui.add_space(8.0);
                if ui.add_enabled(self.receiver.is_none(), egui::Button::new("Open build folder...")).clicked() { self.pick_build(); }
                ui.collapsing("Which files are supported?", |ui| { ui.label("The folder and its subfolders are scanned for linked ELF images (including .elf, .axf and .out), .map and .su. Select firmware to analyze it; supporting files can be previewed. A unique same-name GNU linker map supplies memory capacities automatically. HEX and BIN lack the required metadata."); });
                return;
            };
            if self.view != View::Overview {
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Filter...").desired_width(200.0));
                    if ui.small_button("Clear").clicked() { self.search.clear(); self.selected_file = None; self.kind_filter = "All".into(); }
                    if matches!(self.view, View::Files | View::Symbols) { ui.toggle_value(&mut self.tree, "Directories"); }
                    if self.view == View::Symbols {
                        egui::ComboBox::from_id_salt("symbol_kind").selected_text(&self.kind_filter).width(90.0).show_ui(ui, |ui| {
                            for kind in ["All", "Function", "Global", "Constant", "Label"] { ui.selectable_value(&mut self.kind_filter, kind.into(), kind); }
                        });
                        if self.selected_file.is_some() && ui.small_button("All files").clicked() { self.selected_file = None; }
                    }
                    if self.view == View::Compare {
                        let previous = self.comparison_group;
                        ui.selectable_value(&mut self.comparison_group, 2, "Sections");
                        ui.selectable_value(&mut self.comparison_group, 0, "Files");
                        ui.selectable_value(&mut self.comparison_group, 1, "Symbols");
                        if previous != self.comparison_group {
                            self.search.clear();
                            self.details = None;
                        }
                    }
                });
                ui.separator();
            }
            ui.push_id(self.view.label(), |ui| match self.view {
                View::Overview => self.overview(ui, &a), View::Files => self.files(ui, &a), View::Symbols => self.symbols(ui, &a),
                View::Sections => self.sections(ui, &a), View::MemoryMap => self.memory_map(ui, &a), View::Dependencies => self.dependency_view(ui, &a), View::Stack => self.stack_view(ui), View::Compare => self.compare_view(ui),
            });
        });
    }

    fn show_about_window(&mut self, ctx: &egui::Context) {
        egui::Window::new("About")
            .open(&mut self.show_about)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    // Vector geometry from packaging/icons/snout.svg, scaled for the dialog.
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(80.0, 80.0), egui::Sense::hover());
                    let scale = rect.width() / 256.0;
                    let point = |x, y| rect.min + egui::vec2(x, y) * scale;
                    let background = egui::Color32::from_rgb(24, 33, 43);
                    ui.painter().rect_filled(rect, 48.0 * scale, background);
                    ui.painter().rect_filled(
                        egui::Rect::from_min_max(point(48.0, 60.0), point(208.0, 196.0)),
                        56.0 * scale,
                        egui::Color32::from_rgb(236, 146, 157),
                    );
                    for x in [95.0, 161.0] {
                        ui.painter().add(egui::Shape::ellipse_filled(
                            point(x, 128.0),
                            egui::vec2(18.0, 28.0) * scale,
                            background,
                        ));
                    }
                    ui.add_space(8.0);
                    ui.heading("Rusty's Snout - Firmware Explorer");
                    ui.label(concat!("Version ", env!("CARGO_PKG_VERSION")));
                    ui.add_space(8.0);
                    ui.label("A desktop explorer for embedded firmware memory usage.");
                    ui.add_space(8.0);
                    ui.hyperlink_to(
                        "GitHub repository",
                        "https://github.com/rustypig91/snout-firmware-explorer",
                    );
                });
            });
    }
}
