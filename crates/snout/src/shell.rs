use super::display::display_path;
use super::{egui, Explorer, View};

pub(super) const MIN_TEXT_SIZE: f32 = 12.0;

fn style_popup_menu(ui: &mut egui::Ui) {
    let style = ui.style_mut();
    // egui's menu container replaces the application's button padding.
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 30.0;
    style.visuals.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
    style.visuals.selection.bg_fill = egui::Color32::from_rgb(23, 53, 47);
    style.visuals.selection.stroke = egui::Stroke::NONE;
    for widgets in [
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widgets.weak_bg_fill = egui::Color32::from_rgb(23, 40, 49);
        widgets.bg_fill = egui::Color32::from_rgb(23, 53, 47);
        widgets.bg_stroke = egui::Stroke::NONE;
        widgets.fg_stroke.color = super::views::ACCENT;
    }
}

// Include the recent-folder popup in the app menu hit test between frames.
fn app_menu_popup(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    let bar_id = ui.id();
    let child_id = bar_id.with("recent_menu_rect");
    let size_id = bar_id.with("app_menu_size");
    let mut state = egui::menu::BarState::load(ui.ctx(), bar_id);
    let button = ui.add(
        egui::Button::new("")
            .fill(egui::Color32::TRANSPARENT)
            .min_size(egui::vec2(ui.available_width(), 34.0)),
    );
    button.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Menu")
    });
    // The button owns the full hit area; reserve a separate column for the menu icon.
    ui.painter().text(
        button.rect.left_center() + egui::vec2(34.0, 0.0),
        egui::Align2::LEFT_CENTER,
        "Menu",
        egui::FontId::proportional(15.0),
        ui.style().interact(&button).text_color(),
    );
    let center = egui::pos2(button.rect.left() + 17.0, button.rect.center().y);
    let stroke = egui::Stroke::new(1.5_f32, super::overview::MUTED);
    for offset in [-5.0, 0.0, 5.0] {
        ui.painter().line_segment(
            [
                center + egui::vec2(-8.0, offset),
                center + egui::vec2(8.0, offset),
            ],
            stroke,
        );
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
    state.show(&button, |ui| {
        style_popup_menu(ui);
        contents(ui);
    });
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
        let available_width = (ui.ctx().screen_rect().right() - anchor.x - margin.sum().x).max(1.0);
        let path_width = folders
            .iter()
            .map(|folder| {
                egui::WidgetText::from(display_path(&folder.to_string_lossy()).into_owned())
                    .into_galley(
                        ui,
                        Some(egui::TextWrapMode::Extend),
                        f32::INFINITY,
                        egui::TextStyle::Button,
                    )
                    .size()
                    .x
            })
            .fold(0.0_f32, f32::max);
        let width = (path_width + 20.0).min(available_width);
        let popup = egui::Area::new(id)
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::LEFT_TOP)
            .fixed_pos(anchor)
            .default_width(width + margin.sum().x)
            .sense(egui::Sense::hover())
            .show(ui.ctx(), |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    style_popup_menu(ui);
                    ui.set_width(width);
                    // Keep paths on one line; oversized paths can scroll horizontally.
                    egui::ScrollArea::both()
                        .max_width(width)
                        .max_height(
                            (ui.ctx().screen_rect().bottom() - anchor.y - margin.sum().y).max(30.0),
                        )
                        .show(ui, |ui| {
                            ui.with_layout(
                                egui::Layout::top_down_justified(egui::Align::LEFT),
                                |ui| {
                                    for folder in folders {
                                        let path = folder.to_string_lossy();
                                        if ui
                                            .add(
                                                egui::Button::new(display_path(&path))
                                                    .wrap_mode(egui::TextWrapMode::Extend),
                                            )
                                            .on_hover_text(display_path(&path))
                                            .clicked()
                                        {
                                            selected = Some(folder.clone());
                                        }
                                    }
                                },
                            );
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
    style.visuals.window_stroke = egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(31, 45, 60));
    style.visuals.menu_rounding = egui::Rounding::same(6.0);
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

fn draw_bit_width(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
    let center = rect.center();
    let stroke = egui::Stroke::new(1.0_f32, super::overview::MUTED);
    let painter = ui.painter();
    // A divided register above a width arrow distinguishes word size from CPU type.
    painter.rect_stroke(
        egui::Rect::from_center_size(center + egui::vec2(0.0, -4.0), egui::vec2(18.0, 8.0)),
        1.0,
        stroke,
    );
    for x in [-3.0, 3.0] {
        painter.line_segment(
            [center + egui::vec2(x, -8.0), center + egui::vec2(x, 0.0)],
            stroke,
        );
    }
    painter.line_segment(
        [
            center + egui::vec2(-9.0, 6.0),
            center + egui::vec2(9.0, 6.0),
        ],
        stroke,
    );
    for direction in [-1.0, 1.0] {
        for y in [3.0, 9.0] {
            painter.line_segment(
                [
                    center + egui::vec2(direction * 6.0, y),
                    center + egui::vec2(direction * 9.0, 6.0),
                ],
                stroke,
            );
        }
    }
}

fn reload_button(ui: &mut egui::Ui, enabled: bool, changed: bool) -> egui::Response {
    let response = ui
        .add_enabled(
            enabled,
            egui::Button::new("")
                .fill(egui::Color32::TRANSPARENT)
                .min_size(egui::vec2(32.0, 32.0)),
        )
        .on_hover_text(if changed {
            "Firmware file changed. Refresh to update the analysis (F5)."
        } else {
            "Refresh build folder (F5)"
        })
        .on_disabled_hover_text("Open a build folder to refresh")
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, "Refresh"));
    let center = response.rect.center();
    let color = if changed {
        egui::Color32::from_rgb(245, 184, 75)
    } else if response.hovered() || response.is_pointer_button_down_on() {
        super::views::ACCENT
    } else {
        ui.style().interact(&response).text_color()
    };
    let stroke = egui::Stroke::new(1.8_f32, color);
    let start = std::f32::consts::PI * 0.2;
    let end = std::f32::consts::PI * 1.9;
    let points = (0..=32)
        .map(|step| {
            let angle = start + (end - start) * step as f32 / 32.0;
            center + egui::vec2(angle.cos(), angle.sin()) * 8.0
        })
        .collect();
    ui.painter().add(egui::Shape::line(points, stroke));
    let tip = center + egui::vec2(end.cos(), end.sin()) * 8.0;
    let tangent = egui::vec2(-end.sin(), end.cos());
    let normal = egui::vec2(end.cos(), end.sin());
    ui.painter().add(egui::Shape::line(
        vec![
            tip - tangent * 5.0 + normal * 3.0,
            tip,
            tip - tangent * 5.0 - normal * 3.0,
        ],
        stroke,
    ));
    if changed {
        let dot = response.rect.right_top() + egui::vec2(-5.0, 5.0);
        ui.painter()
            .circle_filled(dot, 4.0, ui.visuals().panel_fill);
        ui.painter().circle_filled(dot, 2.5, color);
    }
    response
}

// Cache the embedded artwork once per egui context for the header and About dialog.
fn draw_app_icon(ui: &mut egui::Ui, size: f32) {
    let id = egui::Id::new("snout_app_icon");
    let cached = ui
        .ctx()
        .data(|data| data.get_temp::<egui::TextureHandle>(id));
    let texture = cached.unwrap_or_else(|| {
        let icon =
            eframe::icon_data::from_png_bytes(include_bytes!("../packaging/icons/snout.png"))
                .expect("bundled Snout icon");
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [icon.width as usize, icon.height as usize],
            &icon.rgba,
        );
        let texture = ui
            .ctx()
            .load_texture("snout_app_icon", image, egui::TextureOptions::LINEAR);
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, texture.clone()));
        texture
    });
    ui.add(egui::Image::new((texture.id(), egui::vec2(size, size))));
}

fn draw_brand(ui: &mut egui::Ui) {
    const ICON_SIZE: f32 = 64.0;
    draw_app_icon(ui, ICON_SIZE);
    let title = egui::WidgetText::from(egui::RichText::new("Rusty's Snout").size(21.0).strong())
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Body,
        );
    let subtitle = egui::WidgetText::from(
        egui::RichText::new("Firmware Explorer")
            .color(super::views::ACCENT)
            .strong(),
    )
    .into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    );
    let text_height = title.size().y + ui.spacing().item_spacing.y + subtitle.size().y;
    ui.vertical(|ui| {
        ui.set_min_height(ICON_SIZE);
        // A vertical child starts at the top of the icon's row. Center the
        // complete two-line title using its measured height.
        ui.add_space(((ICON_SIZE - text_height) * 0.5).max(0.0));
        ui.label(title);
        ui.label(subtitle);
    });
}

#[test]
fn header_title_is_vertically_centered_beside_the_icon() {
    let ctx = egui::Context::default();
    configure_style(&ctx);
    let mut output = egui::FullOutput::default();
    for _ in 0..3 {
        output = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(draw_brand);
            });
        });
    }
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
    let texture_id = ctx.data(|data| {
        data.get_temp::<egui::TextureHandle>(egui::Id::new("snout_app_icon"))
            .unwrap()
            .id()
    });
    let icon = ctx
        .tessellate(output.shapes.clone(), output.pixels_per_point)
        .iter()
        .find_map(|primitive| match &primitive.primitive {
            egui::epaint::Primitive::Mesh(mesh) if mesh.texture_id == texture_id => {
                Some(mesh.calc_bounds())
            }
            _ => None,
        })
        .expect("header icon must be drawn");
    let text = text_rect("Rusty's Snout").union(text_rect("Firmware Explorer"));
    assert!(
        (text.center().y - icon.center().y).abs() < 1.0,
        "title block and icon must share a vertical center: {text:?}, {icon:?}"
    );
}

fn firmware_path_galley(ui: &egui::Ui, path: &str, width: f32) -> std::sync::Arc<egui::Galley> {
    let layout = |text: &str| {
        ui.painter().layout_no_wrap(
            text.to_owned(),
            egui::FontId::proportional(12.0),
            super::overview::MUTED,
        )
    };
    let full = layout(path);
    if full.size().x <= width {
        return full;
    }
    if layout("…").size().x > width {
        return layout("");
    }
    // Search character boundaries so non-ASCII folders remain valid UTF-8.
    // Retain the longest suffix that fits, including the filename at the end.
    let boundaries: Vec<_> = path
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(path.len()))
        .collect();
    let mut left = 0;
    let mut right = boundaries.len() - 1;
    while left < right {
        let middle = left + (right - left) / 2;
        let candidate = layout(&format!("…{}", &path[boundaries[middle]..]));
        if candidate.size().x <= width {
            right = middle;
        } else {
            left = middle + 1;
        }
    }
    layout(&format!("…{}", &path[boundaries[left]..]))
}

#[test]
fn firmware_paths_keep_their_end_when_space_is_limited() {
    let ctx = egui::Context::default();
    configure_style(&ctx);
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            for path in [
                "/home/developer/projects/firmware/build/debug/application.elf",
                "C:\\Users\\Developer\\Projects\\Firmware\\build\\application.elf",
                "/home/开发者/项目/固件/调试/application.elf",
            ] {
                let full = firmware_path_galley(ui, path, 1000.0);
                assert_eq!(full.text(), path);
                for width in [100.0, 180.0, 240.0] {
                    let shortened = firmware_path_galley(ui, path, width);
                    assert!(shortened.size().x <= width);
                    if full.size().x > width {
                        assert!(shortened.text().starts_with('…'));
                    } else {
                        assert_eq!(shortened.text(), path);
                    }
                    assert!(shortened.text().ends_with(".elf"));
                    assert!(path.ends_with(shortened.text().trim_start_matches('…')));
                    if width >= 180.0 {
                        assert!(shortened.text().ends_with("application.elf"));
                    }
                }
                assert!(firmware_path_galley(ui, path, 1.0).text().is_empty());
            }
        });
    });
}

fn draw_firmware_label(
    ui: &egui::Ui,
    pos: egui::Pos2,
    name: &str,
    path: &str,
    width: f32,
    selected: bool,
) {
    let color = if selected {
        super::views::ACCENT
    } else {
        ui.visuals().text_color()
    };
    super::overview::clipped_text(ui, pos, name, width, 15.0, color);
    ui.painter().galley(
        pos + egui::vec2(0.0, 22.0),
        firmware_path_galley(ui, path, width),
        super::overview::MUTED,
    );
}

impl Explorer {
    pub(super) fn firmware_selector(&mut self, ui: &mut egui::Ui) {
        use snout_core::build::ArtifactKind;
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
            draw_firmware_label(
                ui,
                rect.min + egui::vec2(49.0, 8.0),
                &name,
                &path,
                width - 88.0,
                false,
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
                    style_popup_menu(ui);
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
                        // Match the header's two-line layout and keep the virtual
                        // scroll stride equal to the actual button height.
                        let row_height = 54.0;
                        egui::ScrollArea::vertical().max_height(300.0).show_rows(
                            ui,
                            row_height,
                            firmware.len(),
                            |ui, range| {
                                for index in range {
                                    let artifact = firmware[index];
                                    let name = artifact
                                        .path
                                        .file_name()
                                        .unwrap_or(artifact.path.as_os_str())
                                        .to_string_lossy();
                                    let path = artifact.path.to_string_lossy();
                                    let path = display_path(&path);
                                    let active = current.as_ref() == Some(&artifact.path);
                                    let response = ui.add_sized(
                                        egui::vec2(ui.available_width(), row_height),
                                        egui::Button::new("").selected(active).rounding(6.0),
                                    );
                                    response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Button,
                                            ui.is_enabled(),
                                            format!("{name}\n{path}"),
                                        )
                                    });
                                    draw_firmware_label(
                                        ui,
                                        response.rect.min + egui::vec2(10.0, 8.0),
                                        &name,
                                        &path,
                                        response.rect.width() - 20.0,
                                        active,
                                    );
                                    if active {
                                        ui.painter().line_segment(
                                            [
                                                response.rect.left_top() + egui::vec2(0.0, 4.0),
                                                response.rect.left_bottom() - egui::vec2(0.0, 4.0),
                                            ],
                                            egui::Stroke::new(2.0_f32, super::views::ACCENT),
                                        );
                                    }
                                    if response
                                        .on_hover_text(path.as_ref())
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
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
            }
        }
    }

    fn app_menu(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(self.receiver.is_none(), |ui| {
            let bar_id = ui.id();
            app_menu_popup(ui, |ui| {
                if ui
                    .add(
                        egui::Button::new("Open build folder...").shortcut_text(
                            egui::RichText::new("Ctrl+O").color(super::overview::MUTED),
                        ),
                    )
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
                        egui::Button::new("Refresh")
                            .shortcut_text(egui::RichText::new("F5").color(super::overview::MUTED)),
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
                    .add(
                        egui::Button::new("About")
                            .shortcut_text(egui::RichText::new("F1").color(super::overview::MUTED)),
                    )
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
        self.firmware_watch.poll(ctx);
        let snapshot_modal_open = self.snapshot_dialog.is_some() || self.map_warning.is_some();
        self.show_map_warning(ctx);
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
                            ui.label(&analysis.metadata.architecture);
                            ui.add_space(12.0);
                            draw_bit_width(ui);
                            ui.label(format!("{}-bit", analysis.metadata.bitness))
                                .on_hover_text("Firmware word size from the ELF header");
                        });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Keep the action at the far right, beside the baseline badge.
                        if self.receiver.is_some() {
                            ui.add_sized(egui::vec2(32.0, 32.0), egui::Spinner::new());
                            ui.label("Analyzing...");
                        } else if reload_button(
                            ui,
                            self.build.is_some(),
                            self.firmware_watch.changed(),
                        )
                        .clicked()
                        {
                            self.refresh();
                        }
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
                    self.app_menu(ui);
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
                ui.heading("Build files");
                ui.separator();
                self.build_files(ui);
                return;
            }
            let Some(a) = self.analysis.clone() else {
                ui.add_space(24.0); ui.heading("Firmware Explorer");
                ui.label(if self.build.is_some() { "Choose firmware from the ELF dropdown in the header, or open Build files." } else { "Select a build folder to discover firmware, maps and stack reports." });
                ui.add_space(8.0);
                if ui.add_enabled(self.receiver.is_none(), egui::Button::new("Open build folder...")).clicked() { self.pick_build(); }
                ui.collapsing("Which files are supported?", |ui| { ui.label("The folder and its subfolders are scanned for linked ELF images (including .elf, .axf and .out), .map and .su. Select firmware to analyze it; choose map and stack reports in Build files. Automatic map matching compares output sections with the ELF. HEX and BIN lack the required metadata."); });
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
                    draw_app_icon(ui, 80.0);
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
