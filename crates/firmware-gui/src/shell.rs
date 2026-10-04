use super::display::display_path;
use super::{egui, Explorer, View};
use firmware_analysis_core::format_bytes as bytes;

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
    pub(super) fn change_view(&mut self, view: View) {
        if self.view == view {
            return;
        }
        self.view = view;
        self.search.clear();
        self.details = None;
        self.visible_rows = 0;
        self.sort_column = 1;
        self.descending = !matches!(view, View::MemoryMap);
    }

    pub(super) fn show(&mut self, ctx: &egui::Context) {
        if let Some(path) = ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone())) {
            if self.receiver.is_none() {
                self.scan_build(path);
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::O))
            && self.receiver.is_none()
        {
            self.pick_build();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.details = None;
            self.show_notes = false;
        }
        if self.view == View::Overview
            && ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1))
        {
            self.overview_back();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F5)) {
            self.refresh();
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
                    if ui
                        .add_enabled(
                            self.receiver.is_none() && self.build.is_some(),
                            egui::Button::new("Refresh"),
                        )
                        .on_hover_text("Rescan and reload selected firmware (F5)")
                        .clicked()
                    {
                        self.refresh();
                    }
                    ui.add_enabled_ui(self.receiver.is_none(), |ui| {
                        ui.menu_button("Menu", |ui| {
                            if ui.button("Open build folder...").clicked() {
                                ui.close_menu();
                                self.pick_build();
                            }
                            if ui
                                .add_enabled(
                                    self.build.is_some(),
                                    egui::Button::new("Rescan folder"),
                                )
                                .clicked()
                            {
                                ui.close_menu();
                                self.refresh();
                            }
                            if ui
                                .add_enabled(
                                    self.analysis.is_some(),
                                    egui::Button::new("Compare..."),
                                )
                                .on_hover_text(
                                    "Select an older build; deltas show current minus older",
                                )
                                .clicked()
                            {
                                ui.close_menu();
                                self.pick_baseline();
                            }
                            ui.separator();
                            ui.menu_button("Updates", |ui| {
                                ui.label(format!("Snout v{}", env!("CARGO_PKG_VERSION")));
                                if ui
                                    .add_enabled(
                                        self.updates.idle(),
                                        egui::Button::new("Check for updates..."),
                                    )
                                    .clicked()
                                {
                                    self.start_update_check(ctx, true);
                                    ui.close_menu();
                                }
                                if ui
                                    .checkbox(
                                        &mut self.updates.check_on_startup,
                                        "Check on startup",
                                    )
                                    .changed()
                                {
                                    if let Err(error) = self.save_preferences() {
                                        self.error = Some(error.to_string());
                                    }
                                }
                            });
                            ui.menu_button("Layout", |ui| {
                                ui.label(format!(
                                    "{} memory regions configured",
                                    self.options.regions.len()
                                ));
                                if ui.button("Load memory regions...").clicked() {
                                    if let Some(path) = rfd::FileDialog::new()
                                        .add_filter("JSON", &["json"])
                                        .pick_file()
                                    {
                                        self.configure(Some(path));
                                    }
                                    ui.close_menu();
                                }
                                if ui.button("Discover layout from matching map").clicked() {
                                    self.discover_layout();
                                    ui.close_menu();
                                }
                                if ui.button("Use ELF inference").clicked() {
                                    self.configure(None);
                                    ui.close_menu();
                                }
                            });
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
                } else {
                    ui.small("Ready / Select or drop a build folder to begin");
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
            if self.artifact_preview(ui) { return; }
            let Some(a) = self.analysis.clone() else {
                ui.add_space(24.0); ui.heading("Firmware Explorer");
                ui.label(if self.build.is_some() { "Select a firmware image or supporting file in the left pane." } else { "Select a build folder to discover firmware, maps, linker scripts and stack reports." });
                ui.add_space(8.0);
                if ui.add_enabled(self.receiver.is_none(), egui::Button::new("Open build folder...")).clicked() { self.pick_build(); }
                ui.collapsing("Which files are supported?", |ui| { ui.label("The folder and its subfolders are scanned for linked ELF images (including .elf, .axf and .out), .map, .su, .ld/.lds and memory-layout JSON. Select firmware to analyze it; supporting files can be previewed. A unique same-name GNU linker map supplies memory capacities automatically. HEX and BIN lack the required metadata."); });
                return;
            };
            if self.view != View::Overview {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Filter...").desired_width(200.0));
                    if ui.small_button("Clear").clicked() { self.search.clear(); self.selected_file = None; self.kind_filter = "All".into(); }
                    if matches!(self.view, View::Files | View::Symbols) { ui.toggle_value(&mut self.tree, "Directories"); }
                    if self.view == View::Symbols {
                        egui::ComboBox::from_id_salt("symbol_kind").selected_text(&self.kind_filter).width(90.0).show_ui(ui, |ui| {
                            for kind in ["All", "Function", "Global", "Constant", "Label"] { ui.selectable_value(&mut self.kind_filter, kind.into(), kind); }
                        });
                        if self.selected_file.is_some() && ui.small_button("All files").clicked() { self.selected_file = None; }
                    }
                    if self.view == View::Stack {
                        ui.add_enabled_ui(self.receiver.is_none(), |ui| {
                            if ui.button("Load all build reports").on_hover_text("Load all discovered .su files in the selected build folder. Reports may belong to different targets; check their paths.").clicked() { self.load_build_stack(); }
                            if ui.button("Open .su...").clicked() { self.pick_stack(false); }
                            if ui.button("Scan folder...").clicked() { self.pick_stack(true); }
                        });
                    }
                    if self.view == View::Compare {
                        ui.selectable_value(&mut self.comparison_symbols, false, "Files");
                        ui.selectable_value(&mut self.comparison_symbols, true, "Symbols");
                    }
                });
                ui.separator();
            }
            ui.push_id(self.view.label(), |ui| match self.view {
                View::Overview => self.overview(ui, &a), View::Files => self.files(ui, &a), View::Symbols => self.symbols(ui, &a),
                View::Sections => self.sections(ui, &a), View::MemoryMap => self.memory_map(ui, &a), View::Stack => self.stack_view(ui), View::Compare => self.compare_view(ui),
            });
        });
    }
}
