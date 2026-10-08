use super::display::display_path;
use super::{egui, Explorer, View};

pub(super) const MIN_TEXT_SIZE: f32 = 12.0;

// Include the recent-folder popup in the Settings menu hit test between frames.
fn settings_popup(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    let bar_id = ui.id();
    let child_id = bar_id.with("recent_menu_rect");
    let size_id = bar_id.with("settings_menu_size");
    let mut state = egui::menu::BarState::load(ui.ctx(), bar_id);
    let button = ui.add(
        egui::Button::new("")
            .fill(egui::Color32::TRANSPARENT)
            .min_size(egui::vec2(ui.available_width(), 34.0)),
    );
    button.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Settings")
    });
    // The button owns the full hit area; reserve a separate column for the gear.
    ui.painter().text(
        button.rect.left_center() + egui::vec2(34.0, 0.0),
        egui::Align2::LEFT_CENTER,
        "Settings",
        egui::FontId::proportional(15.0),
        ui.style().interact(&button).text_color(),
    );
    let center = egui::pos2(button.rect.left() + 17.0, button.rect.center().y);
    let stroke = egui::Stroke::new(1.5_f32, super::overview::MUTED);
    ui.painter().circle_stroke(center, 5.5, stroke);
    ui.painter().circle_stroke(center, 2.0, stroke);
    for tooth in 0..8 {
        let angle = tooth as f32 * std::f32::consts::TAU / 8.0;
        let direction = egui::vec2(angle.cos(), angle.sin());
        ui.painter()
            .line_segment([center + direction * 5.5, center + direction * 8.0], stroke);
    }
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
    if let Some(root) = state.as_ref() {
        let mut menu = root.menu_state.write();
        // Hovering the button recreates MenuRoot with an empty rect. Keep the
        // measured popup size separately, including while a child menu is open.
        let size = ui
            .ctx()
            .data(|data| data.get_temp::<egui::Vec2>(size_id))
            .unwrap_or(menu.rect.size());
        let height = size.y;
        menu.rect = egui::Rect::from_min_size(
            egui::pos2(
                button.rect.left(),
                (button.rect.top() - height - ui.spacing().menu_spacing)
                    .max(ui.ctx().screen_rect().top()),
            ),
            size,
        );
    }
    state.show(&button, contents);
    if let Some(root) = state.as_ref() {
        ui.ctx().data_mut(|data| {
            data.insert_temp(size_id, root.menu_state.read().rect.size());
        });
    }
    if state.as_ref().is_none() {
        ui.ctx().data_mut(|data| {
            data.insert_temp(bar_id.with("recent_menu"), false);
        });
    }
    state.store(ui.ctx(), bar_id);
}

fn right_recent_menu(
    ui: &mut egui::Ui,
    folders: &[std::path::PathBuf],
    bar_id: egui::Id,
) -> Option<std::path::PathBuf> {
    let id = bar_id.with("recent_menu");
    let button = ui.add_enabled(
        !folders.is_empty(),
        egui::Button::new("Open recent build folder ▶"),
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
            parent.right() + margin.right + ui.spacing().menu_spacing,
            button.rect.top() - margin.top,
        );
        // Bound the popup to the space right of its parent. Otherwise long
        // paths make Area's screen constraint move it over the parent menu.
        let width = (ui.ctx().screen_rect().right() - anchor.x - margin.sum().x).clamp(1.0, 400.0);
        let popup = egui::Area::new(id)
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::LEFT_TOP)
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
    style.visuals.panel_fill = egui::Color32::from_rgb(11, 18, 26);
    style.visuals.window_fill = egui::Color32::from_rgb(16, 25, 35);
    style.visuals.extreme_bg_color = egui::Color32::from_rgb(8, 14, 21);
    style.visuals.faint_bg_color = egui::Color32::from_white_alpha(4);
    style.visuals.override_text_color = Some(egui::Color32::from_rgb(207, 220, 238));
    style.visuals.selection.bg_fill = egui::Color32::from_rgb(23, 53, 47);
    style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, super::views::ACCENT);
    style.visuals.widgets.noninteractive.bg_stroke =
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(31, 45, 60));
    style.visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(23, 34, 47);
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    for widgets in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        widgets.rounding = egui::Rounding::same(6.0);
        widgets.expansion = 0.0;
    }
    style.spacing.item_spacing = egui::vec2(10.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 23.0;
    for text_style in [
        egui::TextStyle::Body,
        egui::TextStyle::Button,
        egui::TextStyle::Monospace,
    ] {
        if let Some(font) = style.text_styles.get_mut(&text_style) {
            font.size = 14.0;
        }
    }
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(17.0));
    for font in style.text_styles.values_mut() {
        font.size = font.size.max(MIN_TEXT_SIZE);
    }
    ctx.set_style(style);
}

// Use the application's existing snout motif as vector geometry so the header
// remains crisp at every display scale and needs no external image assets.
fn draw_chip(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
    let stroke = egui::Stroke::new(1.0_f32, super::overview::MUTED);
    ui.painter().rect_stroke(rect.shrink(4.0), 1.0, stroke);
    for offset in [-4.0, 0.0, 4.0] {
        let center = rect.center();
        for (start, end) in [
            (egui::vec2(offset, -10.0), egui::vec2(offset, -6.0)),
            (egui::vec2(offset, 6.0), egui::vec2(offset, 10.0)),
            (egui::vec2(-10.0, offset), egui::vec2(-6.0, offset)),
            (egui::vec2(6.0, offset), egui::vec2(10.0, offset)),
        ] {
            ui.painter()
                .line_segment([center + start, center + end], stroke);
        }
    }
}

fn draw_brand(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(38.0, 44.0), egui::Sense::hover());
    let center = rect.center();
    let stroke = egui::Stroke::new(1.8_f32, super::views::ACCENT);
    ui.painter().rect_stroke(
        egui::Rect::from_center_size(center, egui::vec2(33.0, 29.0)),
        12.0,
        stroke,
    );
    ui.painter().rect_stroke(
        egui::Rect::from_center_size(center + egui::vec2(0.0, 5.0), egui::vec2(19.0, 12.0)),
        6.0,
        stroke,
    );
    for x in [-7.0, 7.0] {
        ui.painter()
            .circle_filled(center + egui::vec2(x, -5.0), 2.0, super::views::ACCENT);
        ui.painter().line_segment(
            [
                center + egui::vec2(x * 1.8, -10.0),
                center + egui::vec2(x * 2.0, -20.0),
            ],
            stroke,
        );
    }
    for x in [-4.0, 4.0] {
        ui.painter()
            .circle_filled(center + egui::vec2(x, 5.0), 1.5, super::views::ACCENT);
    }
    ui.vertical(|ui| {
        ui.label(egui::RichText::new("Rusty's Snout").size(21.0).strong());
        ui.label(
            egui::RichText::new("Firmware Explorer")
                .color(super::views::ACCENT)
                .strong(),
        );
    });
}

impl Explorer {
    pub(super) fn firmware_selector(&mut self, ui: &mut egui::Ui) {
        use firmware_analysis_core::build::ArtifactKind;
        let current = self
            .analysis
            .as_ref()
            .map(|a| std::path::PathBuf::from(&a.path));
        let name = current
            .as_ref()
            .map(|path| {
                path.file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
                    .into_owned()
            })
            .unwrap_or_else(|| "Select ELF firmware…".into());
        let path = current
            .as_ref()
            .map(|path| display_path(&path.to_string_lossy()).into_owned())
            .unwrap_or_else(|| "Open a build folder to select firmware".into());
        let popup_id = ui.make_persistent_id("current_elf");
        let width = (ui.ctx().screen_rect().width() - 560.0).clamp(240.0, 470.0);
        let mut selected = None;
        ui.add_enabled_ui(self.receiver.is_none(), |ui| {
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(width, 54.0), egui::Sense::click());
            let fill = if response.hovered() {
                egui::Color32::from_rgb(29, 43, 58)
            } else {
                egui::Color32::from_rgb(23, 34, 47)
            };
            ui.painter().rect(
                rect,
                6.0,
                fill,
                egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(34, 49, 65)),
            );
            let stroke = egui::Stroke::new(1.5_f32, super::overview::MUTED);
            let origin = rect.left_top() + egui::vec2(18.0, 17.0);
            let points = [
                origin,
                origin + egui::vec2(10.0, 0.0),
                origin + egui::vec2(15.0, 5.0),
                origin + egui::vec2(15.0, 20.0),
                origin + egui::vec2(0.0, 20.0),
                origin,
            ];
            ui.painter().add(egui::Shape::line(points.to_vec(), stroke));
            ui.painter().line_segment(
                [
                    origin + egui::vec2(10.0, 0.0),
                    origin + egui::vec2(10.0, 5.0),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    origin + egui::vec2(10.0, 5.0),
                    origin + egui::vec2(15.0, 5.0),
                ],
                stroke,
            );
            for y in [10.0, 14.0] {
                ui.painter().line_segment(
                    [origin + egui::vec2(4.0, y), origin + egui::vec2(10.0, y)],
                    stroke,
                );
            }
            super::overview::clipped_text(
                ui,
                rect.min + egui::vec2(49.0, 8.0),
                &name,
                width - 88.0,
                15.0,
                ui.visuals().text_color(),
            );
            super::overview::clipped_text(
                ui,
                rect.min + egui::vec2(49.0, 30.0),
                &path,
                width - 88.0,
                12.0,
                super::overview::MUTED,
            );
            let arrow = rect.right_center() - egui::vec2(22.0, 0.0);
            ui.painter().line_segment(
                [arrow + egui::vec2(-5.0, -2.0), arrow + egui::vec2(0.0, 3.0)],
                stroke,
            );
            ui.painter().line_segment(
                [arrow + egui::vec2(0.0, 3.0), arrow + egui::vec2(5.0, -2.0)],
                stroke,
            );
            let response = response
                .on_hover_text(&path)
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked() {
                ui.memory_mut(|memory| memory.toggle_popup(popup_id));
            }
            egui::popup::popup_below_widget(
                ui,
                popup_id,
                &response,
                egui::popup::PopupCloseBehavior::CloseOnClickOutside,
                |ui| {
                    ui.set_width(width - 16.0);
                    if let Some(build) = &self.build {
                        let firmware: Vec<_> = build
                            .artifacts
                            .iter()
                            .filter(|artifact| artifact.kind == ArtifactKind::Firmware)
                            .collect();
                        if firmware.is_empty() {
                            ui.weak("No firmware images found in this build folder.");
                        }
                        // Frame-free firmware buttons use the text height or minimum
                        // interaction height. Virtual rows must use the same stride.
                        let row_height = ui
                            .spacing()
                            .interact_size
                            .y
                            .max(ui.text_style_height(&egui::TextStyle::Button));
                        egui::ScrollArea::vertical().max_height(300.0).show_rows(
                            ui,
                            row_height,
                            firmware.len(),
                            |ui, range| {
                                for index in range {
                                    let artifact = firmware[index];
                                    let relative = artifact
                                        .path
                                        .strip_prefix(&build.root)
                                        .unwrap_or(&artifact.path);
                                    if ui
                                        .add(
                                            egui::Button::new(display_path(
                                                &relative.to_string_lossy(),
                                            ))
                                            .frame(false)
                                            .selected(current.as_ref() == Some(&artifact.path))
                                            .truncate(),
                                        )
                                        .on_hover_text(display_path(
                                            &artifact.path.to_string_lossy(),
                                        ))
                                        .clicked()
                                    {
                                        selected = Some(artifact.clone());
                                        ui.memory_mut(|memory| memory.close_popup());
                                    }
                                }
                            },
                        );
                    } else if current.is_some() {
                        ui.label(&path);
                    } else {
                        ui.weak("Open a build folder first.");
                    }
                },
            );
        });
        if let Some(artifact) = selected {
            if current.as_ref() != Some(&artifact.path) {
                self.select_artifact(artifact);
            } else {
                self.preview = None;
            }
        }
    }

    fn settings_menu(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(self.receiver.is_none(), |ui| {
            let bar_id = ui.id();
            settings_popup(ui, |ui| {
                if ui
                    .add(egui::Button::new("Open build folder...").shortcut_text(
                        egui::RichText::new("Ctrl+O").color(egui::Color32::from_gray(145)),
                    ))
                    .clicked()
                {
                    ui.close_menu();
                    self.pick_build();
                }
                if let Some(folder) = right_recent_menu(ui, &self.recent_build_folders, bar_id) {
                    self.scan_build(folder);
                }
                if ui
                    .add_enabled(
                        self.build.is_some(),
                        egui::Button::new("Refresh").shortcut_text(
                            egui::RichText::new("F5").color(egui::Color32::from_gray(145)),
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
                        self.preferences_file.is_some(),
                        egui::Button::new("Open configuration folder"),
                    )
                    .on_hover_text(
                        "Open Snout's workspace settings and snapshots in your file explorer",
                    )
                    .clicked()
                {
                    ui.close_menu();
                    if let Err(error) = self.open_configuration_folder() {
                        self.error = Some(error);
                    }
                }
                ui.separator();
                if ui
                    .add_enabled(self.updates.idle(), egui::Button::new("Check for updates"))
                    .on_disabled_hover_text("An update check or installation is in progress")
                    .clicked()
                {
                    self.start_update_check(ui.ctx(), true);
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
                        self.error = Some(format!("Could not save update preferences: {error}"));
                    }
                }
                ui.separator();
                if ui
                    .add_enabled(self.analysis.is_some(), egui::Button::new("Analysis notes"))
                    .on_hover_text("Warnings, missing information and limitations")
                    .clicked()
                {
                    self.show_notes = !self.show_notes;
                    ui.close_menu();
                }
                if ui
                    .button("Support developer")
                    .on_hover_text("Opens Buy Me a Coffee in your browser")
                    .clicked()
                {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(
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
    }

    pub(super) fn navigation(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        for view in View::ALL {
            let active = self.view == view;
            let response = ui
                .scope(|ui| {
                    let visuals = &mut ui.style_mut().visuals;
                    visuals.selection.bg_fill = egui::Color32::from_rgb(23, 53, 47);
                    visuals.selection.stroke = egui::Stroke::NONE;
                    visuals.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                    visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                    visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(23, 40, 49);
                    visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
                    let text = egui::RichText::new(view.label()).size(15.0);
                    let text = if active {
                        text.strong().color(super::views::ACCENT)
                    } else {
                        text
                    };
                    ui.add(
                        egui::Button::new(text)
                            .selected(active)
                            .min_size(egui::vec2(ui.available_width(), 34.0)),
                    )
                })
                .inner
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(view.tooltip());
            if active {
                ui.painter().line_segment(
                    [
                        response.rect.left_top() + egui::vec2(0.0, 4.0),
                        response.rect.left_bottom() - egui::vec2(0.0, 4.0),
                    ],
                    egui::Stroke::new(2.0_f32, super::views::ACCENT),
                );
            }
            if response.clicked() {
                self.change_view(view);
            }
        }
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(6.0);
    }

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
        if self.view == View::BuildFiles && view != View::BuildFiles {
            self.preview = None;
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
        egui::TopBottomPanel::top("workbench_tabs")
            .frame(
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(11, 18, 26))
                    .inner_margin(egui::Margin::symmetric(16.0, 14.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.horizontal(|ui| {
                        draw_brand(ui);
                        ui.add_space(22.0);
                        self.firmware_selector(ui);
                    });
                    if let Some(analysis) = &self.analysis {
                        ui.horizontal(|ui| {
                            draw_chip(ui);
                            ui.label("ELF");
                            ui.add_space(12.0);
                            draw_chip(ui);
                            ui.label(&analysis.metadata.architecture);
                            ui.add_space(12.0);
                            ui.label(format!("{}-bit", analysis.metadata.bitness));
                        });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(name) = self.snapshot_label().map(str::to_owned) {
                            let label = format!("Baseline: {name}");
                            let text = egui::RichText::new(&label)
                                .small()
                                .color(super::views::ACCENT);
                            if ui
                                .add_sized(
                                    egui::vec2(180.0_f32.min(ui.available_width()), 28.0),
                                    egui::Button::new(text).selected(true).truncate(),
                                )
                                .on_hover_text(format!(
                                    "{label}\nClick to clear baseline comparison"
                                ))
                                .clicked()
                            {
                                if let Err(error) = self.select_snapshot(None) {
                                    self.snapshot_error = Some(error);
                                }
                            }
                        }
                        if self.receiver.is_some() {
                            ui.spinner();
                            ui.label("Analyzing...");
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
        egui::SidePanel::left("workbench_navigation")
            .exact_width(165.0)
            .show(ctx, |ui| {
                let bounds = ui.available_rect_before_wrap();
                let settings = egui::Rect::from_min_max(
                    egui::pos2(bounds.left(), bounds.bottom() - 34.0),
                    bounds.right_bottom(),
                );
                let navigation = egui::Rect::from_min_max(
                    bounds.left_top(),
                    egui::pos2(bounds.right(), settings.top() - 12.0),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(navigation), |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.navigation(ui));
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(settings), |ui| {
                    self.settings_menu(ui);
                });
            });
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
            if self.view == View::Baselines { self.baselines_view(ui); return; }
            if self.view == View::BuildFiles {
                ui.horizontal(|ui| {
                    ui.heading("Build files");
                    if ui.button("Back to Overview").clicked() { self.change_view(View::Overview); }
                });
                ui.separator();
                if self.preview.is_some() {
                    if ui.button("Back to file selection").clicked() { self.preview = None; }
                    if self.artifact_preview(ui) { return; }
                }
                self.build_files(ui);
                return;
            }
            if self.view == View::Overview && self.artifact_preview(ui) { return; }
            let Some(a) = self.analysis.clone() else {
                ui.add_space(24.0); ui.heading("Firmware Explorer");
                ui.label(if self.build.is_some() { "Choose firmware from the ELF dropdown in the header, or open Build files." } else { "Select a build folder to discover firmware, maps and stack reports." });
                ui.add_space(8.0);
                if ui.add_enabled(self.receiver.is_none(), egui::Button::new("Open build folder...")).clicked() { self.pick_build(); }
                ui.collapsing("Which files are supported?", |ui| { ui.label("The folder and its subfolders are scanned for linked ELF images (including .elf, .axf and .out), .map and .su. Select firmware to analyze it; supporting files can be previewed. A unique same-name GNU linker map supplies memory capacities automatically. HEX and BIN lack the required metadata."); });
                return;
            };
            if !matches!(self.view, View::Overview | View::Memory) {
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Filter...").desired_width(200.0));
                    if ui.small_button("Clear").clicked() { self.search.clear(); self.selected_file = None; self.kind_filter = "All".into(); }
                    if self.diffs_active() && matches!(self.view, View::Symbols | View::Sections | View::MemoryMap) {
                        let selected = &mut self.show_address_changes[self.view as usize];
                        if ui.add(egui::Button::new("Show address changes").selected(*selected)).on_hover_text("Include entries whose only change is their address").clicked() { *selected = !*selected; }
                    }
                    if matches!(self.view, View::Files | View::Symbols) { ui.toggle_value(&mut self.tree, "Directories"); }
                    if self.view == View::Symbols {
                        egui::ComboBox::from_id_salt("symbol_kind").selected_text(&self.kind_filter).width(90.0).show_ui(ui, |ui| {
                            for kind in ["All", "Function", "Global", "Constant", "Label"] { ui.selectable_value(&mut self.kind_filter, kind.into(), kind); }
                        });
                        if self.selected_file.is_some() && ui.small_button("All files").clicked() { self.selected_file = None; }
                    }
                });
                ui.separator();
            }
            if self.view == View::Memory && self.diffs_active() {
                ui.weak("Current firmware only · Hex viewer does not support baseline comparison");
            }
            ui.push_id(self.view.label(), |ui| match self.view {
                View::Overview => self.overview(ui, &a), View::Files => self.files(ui, &a), View::Symbols => self.symbols(ui, &a),
                View::Sections => self.sections(ui, &a), View::MemoryMap => self.memory_map(ui, &a), View::Dependencies => self.dependency_view(ui, &a), View::Stack => self.stack_view(ui), View::Memory => self.memory_view(ui, &a), View::BuildFiles => self.build_files(ui), View::Baselines => self.baselines_view(ui),
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
