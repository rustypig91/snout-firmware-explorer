use super::egui;

pub(super) const CONTROLS_WIDTH: f32 = 156.0;

#[derive(Clone, Copy, Default)]
enum NativePointerCapture {
    ReleasePending(egui::Pos2),
    #[default]
    SuppressRelease,
}

fn capture_id(ctx: &egui::Context) -> egui::Id {
    egui::Id::new(("native_window_pointer_capture", ctx.viewport_id()))
}

fn hand_off_pointer(ctx: &egui::Context, pos: egui::Pos2) {
    let id = capture_id(ctx);
    ctx.data_mut(|data| data.insert_temp(id, NativePointerCapture::ReleasePending(pos)));
    ctx.stop_dragging();
    ctx.request_repaint();
}

pub(super) fn raw_input_hook(ctx: &egui::Context, input: &mut egui::RawInput) {
    let id = capture_id(ctx);
    let Some(capture) = ctx.data_mut(|data| data.remove_temp::<NativePointerCapture>(id)) else {
        return;
    };
    let mut injected = false;
    if let NativePointerCapture::ReleasePending(pos) = capture {
        let first_button = input.events.iter().find_map(|event| match event {
            egui::Event::PointerButton {
                button: egui::PointerButton::Primary,
                pressed,
                ..
            } => Some(*pressed),
            _ => None,
        });
        if first_button == Some(false) {
            // The backend delivered the real release; keep normal click history.
            return;
        }
        // Native move/resize can consume mouse-up. Release egui's press at the
        // original handle, before new input, so unrelated controls cannot click.
        input.events.insert(
            0,
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: ctx.input(|i| i.modifiers),
            },
        );
        injected = true;
        // The release frame may still highlight the old handle. Repaint once
        // more to show hover even if the pointer is now stationary.
        ctx.request_repaint();
        // Skip the release we just inserted when filtering a delayed real one.
    }
    let mut suppress = true;
    let mut index = 0;
    input.events.retain(|event| {
        let synthetic = injected && index == 0;
        index += 1;
        if synthetic {
            return true;
        }
        match event {
            egui::Event::PointerButton {
                button: egui::PointerButton::Primary,
                pressed: true,
                ..
            } => {
                suppress = false;
                true
            }
            egui::Event::PointerButton {
                button: egui::PointerButton::Primary,
                pressed: false,
                ..
            } if suppress => false,
            _ => true,
        }
    });
    if suppress {
        ctx.data_mut(|data| data.insert_temp(id, NativePointerCapture::SuppressRelease));
    }
}

pub(super) fn header_drag(ui: &mut egui::Ui, rect: egui::Rect) {
    // Register the background first so selectors and buttons take precedence.
    let response = ui.interact(rect, ui.id().with("window_drag"), egui::Sense::click());
    let armed_id = ui.id().with("window_drag_armed");
    if ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary)) {
        ui.ctx().data_mut(|data| {
            data.insert_temp(armed_id, response.is_pointer_button_down_on());
        });
    }
    let armed = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(armed_id).unwrap_or(false));
    if !ui.input(|i| i.pointer.primary_down()) {
        ui.ctx().data_mut(|data| data.remove::<bool>(armed_id));
    }
    if response.double_clicked() {
        let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    } else if armed && ui.input(|i| i.pointer.primary_down() && i.pointer.is_decidedly_dragging()) {
        // Leave ordinary clicks with egui. Handing off on press would inject
        // a release on the next repaint, corrupting double-click timing.
        // Keep click-only sense so a press on an overlaid button cannot also
        // capture the header as a drag target.
        ui.ctx().data_mut(|data| data.remove::<bool>(armed_id));
        hand_off_pointer(
            ui.ctx(),
            response.interact_pointer_pos().unwrap_or(rect.center()),
        );
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

pub(super) fn controls(ui: &mut egui::Ui, rect: egui::Rect) {
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            for (name, command, icon) in [
                ("Close", egui::ViewportCommand::Close, 0),
                (
                    if maximized { "Restore" } else { "Maximize" },
                    egui::ViewportCommand::Maximized(!maximized),
                    1,
                ),
                ("Minimize", egui::ViewportCommand::Minimized(true), 2),
            ] {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(40.0, 32.0), egui::Sense::click());
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name)
                });
                let active = response.is_pointer_button_down_on();
                let hovered = response.hovered();
                let fill = if (hovered || active) && icon == 0 {
                    egui::Color32::from_rgb(if active { 150 } else { 190 }, 45, 57)
                } else if active {
                    egui::Color32::from_rgb(34, 57, 72)
                } else if hovered {
                    egui::Color32::from_rgb(28, 43, 58)
                } else {
                    egui::Color32::from_rgb(16, 25, 35)
                };
                let stroke = egui::Stroke::new(
                    1.5,
                    if hovered || active {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::from_rgb(169, 189, 209)
                    },
                );
                ui.painter().rect(
                    rect,
                    5,
                    fill,
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(31, 45, 60)),
                    egui::StrokeKind::Inside,
                );
                let center = rect.center();
                match icon {
                    0 => {
                        for sign in [-1.0, 1.0] {
                            ui.painter().line_segment(
                                [
                                    center + egui::vec2(-5.0, -5.0 * sign),
                                    center + egui::vec2(5.0, 5.0 * sign),
                                ],
                                stroke,
                            );
                        }
                    }
                    1 => {
                        let square = egui::Rect::from_center_size(center, egui::vec2(11.0, 11.0));
                        if maximized {
                            ui.painter().rect_stroke(
                                square.translate(egui::vec2(2.0, -2.0)),
                                0,
                                stroke,
                                egui::StrokeKind::Inside,
                            );
                            ui.painter().rect_filled(
                                square.translate(egui::vec2(-1.0, 1.0)),
                                0,
                                fill,
                            );
                        }
                        ui.painter().rect_stroke(
                            square.translate(if maximized {
                                egui::vec2(-1.0, 1.0)
                            } else {
                                egui::Vec2::ZERO
                            }),
                            0,
                            stroke,
                            egui::StrokeKind::Inside,
                        );
                    }
                    _ => {
                        ui.painter().line_segment(
                            [
                                center + egui::vec2(-5.5, 2.0),
                                center + egui::vec2(5.5, 2.0),
                            ],
                            stroke,
                        );
                    }
                }
                if response.clicked() {
                    ui.ctx().send_viewport_cmd(command);
                }
                response.on_hover_text(name);
            }
            let (divider, _) = ui.allocate_exact_size(egui::vec2(6.0, 32.0), egui::Sense::hover());
            ui.painter().line_segment(
                [divider.center_top(), divider.center_bottom()],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(31, 45, 60)),
            );
        },
    );
}

pub(super) fn resize_edges(ctx: &egui::Context) {
    if ctx.input(|i| {
        i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)
    }) {
        return;
    }
    let rect = ctx.viewport_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("window_border"),
    ));
    painter.rect_stroke(
        rect,
        0,
        egui::Stroke::new(1.0, egui::Color32::from_rgb(31, 45, 60)),
        egui::StrokeKind::Inside,
    );
    // Wide invisible edge bands are easier to grab than the painted border.
    // Corner reach follows those bands rather than covering nearby content.
    let edge = 8.0;
    let corner = 28.0;
    use egui::{CursorIcon as C, ResizeDirection as D};
    for (index, (side, offset, size)) in [
        (
            D::North,
            egui::vec2(0.0, 0.0),
            egui::vec2(rect.width(), edge),
        ),
        (
            D::South,
            egui::vec2(0.0, rect.height() - edge),
            egui::vec2(rect.width(), edge),
        ),
        (
            D::West,
            egui::vec2(0.0, 0.0),
            egui::vec2(edge, rect.height()),
        ),
        (
            D::East,
            egui::vec2(rect.width() - edge, 0.0),
            egui::vec2(edge, rect.height()),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let pointer = ctx
            .input(|i| i.pointer.interact_pos())
            .unwrap_or(rect.center());
        let west = pointer.x < rect.left() + corner;
        let east = pointer.x > rect.right() - corner;
        let north = pointer.y < rect.top() + corner;
        let south = pointer.y > rect.bottom() - corner;
        let direction = match side {
            D::North if west => D::NorthWest,
            D::North if east => D::NorthEast,
            D::South if west => D::SouthWest,
            D::South if east => D::SouthEast,
            D::West if north => D::NorthWest,
            D::West if south => D::SouthWest,
            D::East if north => D::NorthEast,
            D::East if south => D::SouthEast,
            _ => side,
        };
        let cursor = match direction {
            D::NorthWest | D::SouthEast => C::ResizeNwSe,
            D::NorthEast | D::SouthWest => C::ResizeNeSw,
            D::North | D::South => C::ResizeVertical,
            D::West | D::East => C::ResizeHorizontal,
        };
        egui::Area::new(egui::Id::new(("window_resize", index)))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min + offset)
            .constrain(false)
            .movable(false)
            .show(ctx, |ui| {
                let (_, response) = ui.allocate_exact_size(size, egui::Sense::drag());
                let response = response.on_hover_cursor(cursor);
                if response.is_pointer_button_down_on()
                    && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary))
                {
                    hand_off_pointer(
                        ctx,
                        response
                            .interact_pointer_pos()
                            .unwrap_or(response.rect.center()),
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, maximized: bool) -> egui::FullOutput {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(300.0, 100.0),
            )),
            events,
            ..Default::default()
        };
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .maximized = Some(maximized);
        raw_input_hook(ctx, &mut input);
        crate::tests::run_ui(ctx, input, |ui| {
            let rect =
                egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(CONTROLS_WIDTH, 32.0));
            header_drag(
                ui,
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 64.0)),
            );
            controls(ui, rect);
        })
    }

    #[test]
    fn buttons_highlight_and_send_window_commands_without_dragging() {
        for (x, maximized, command, color) in [
            (
                156.0,
                false,
                egui::ViewportCommand::Close,
                egui::Color32::from_rgb(190, 45, 57),
            ),
            (
                106.0,
                false,
                egui::ViewportCommand::Maximized(true),
                egui::Color32::from_rgb(28, 43, 58),
            ),
            (
                106.0,
                true,
                egui::ViewportCommand::Maximized(false),
                egui::Color32::from_rgb(28, 43, 58),
            ),
            (
                56.0,
                false,
                egui::ViewportCommand::Minimized(true),
                egui::Color32::from_rgb(28, 43, 58),
            ),
        ] {
            let ctx = egui::Context::default();
            let pos = egui::pos2(x, 36.0);
            frame(&ctx, vec![], maximized);
            let output = frame(&ctx, vec![egui::Event::PointerMoved(pos)], maximized);
            assert!(
                output.shapes.iter().any(
                    |shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == color)
                ),
                "Hover highlight missing for {command:?}"
            );
            for pressed in [true, false] {
                let output = frame(
                    &ctx,
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    maximized,
                );
                let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
                assert!(!commands.contains(&egui::ViewportCommand::StartDrag));
                if !pressed {
                    assert!(
                        commands.contains(&command),
                        "Missing {command:?}: {commands:?}"
                    );
                }
            }
            // Dragging away from a control must not move the window behind it.
            frame(
                &ctx,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
                maximized,
            );
            let output = frame(
                &ctx,
                vec![egui::Event::PointerMoved(pos + egui::vec2(20.0, 0.0))],
                maximized,
            );
            assert!(!output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .contains(&egui::ViewportCommand::StartDrag));
        }
    }

    #[test]
    fn header_background_starts_native_drag() {
        let ctx = egui::Context::default();
        frame(&ctx, vec![], false);
        let pos = egui::pos2(230.0, 36.0);
        let output = frame(
            &ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            false,
        );
        assert!(!output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::StartDrag));
        let output = frame(
            &ctx,
            vec![egui::Event::PointerMoved(pos + egui::vec2(12.0, 0.0))],
            false,
        );
        assert!(output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::StartDrag));
        // Recover from a consumed native release, then click the header once.
        // A completed drag must not seed double-click history.
        frame(&ctx, vec![], false);
        let pos = pos + egui::vec2(12.0, 0.0);
        for pressed in [true, false] {
            let output = frame(
                &ctx,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
                false,
            );
            assert!(!output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .contains(&egui::ViewportCommand::Maximized(true)));
        }
    }
    #[test]
    fn hovering_and_clicking_recover_after_native_move_or_resize() {
        for (pos, command) in [
            (egui::pos2(950.0, 46.0), egui::ViewportCommand::StartDrag),
            (
                egui::pos2(2.0, 400.0),
                egui::ViewportCommand::BeginResize(egui::ResizeDirection::West),
            ),
            (
                egui::pos2(1278.0, 818.0),
                egui::ViewportCommand::BeginResize(egui::ResizeDirection::SouthEast),
            ),
        ] {
            for release_delivered in [false, true] {
                let ctx = egui::Context::default();
                let mut app = crate::Explorer::default();
                let mut frame = |events| {
                    let mut input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280.0, 820.0),
                        )),
                        events,
                        ..Default::default()
                    };
                    // Exercise the same hook eframe invokes before egui processes input.
                    eframe::App::raw_input_hook(&mut app, &ctx, &mut input);
                    crate::tests::run_ui(&ctx, input, |ui| app.show(ui))
                };
                let button = |pos, pressed| egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                frame(vec![]);
                frame(vec![]);
                let mut output = frame(vec![egui::Event::PointerMoved(pos), button(pos, true)]);
                if command == egui::ViewportCommand::StartDrag {
                    output = frame(vec![egui::Event::PointerMoved(pos + egui::vec2(12.0, 0.0))]);
                }
                assert!(output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .contains(&command));
                // Native move/resize can consume mouse-up, or deliver it normally.
                let close = egui::pos2(1244.0, 46.0);
                let mut events = vec![egui::Event::PointerMoved(close)];
                if release_delivered {
                    events.push(button(close, false));
                }
                let release_output = frame(events);
                assert!(!release_output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .contains(&egui::ViewportCommand::Close));
                let output = frame(vec![]);
                let highlighted = |output: &egui::FullOutput| {
                    output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == egui::Color32::from_rgb(190, 45, 57)))
                };
                assert!(
                    highlighted(&output),
                    "Hover after {command:?}, release delivered={release_delivered}"
                );
                assert!(!output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .contains(&egui::ViewportCommand::Close));
                if !release_delivered {
                    // A late real release must not click the button or count twice.
                    let output = frame(vec![button(close, false)]);
                    assert!(highlighted(&output));
                    assert!(!output.viewport_output[&egui::ViewportId::ROOT]
                        .commands
                        .contains(&egui::ViewportCommand::Close));
                }
                frame(vec![button(close, true)]);
                let output = frame(vec![button(close, false)]);
                assert!(
                    output.viewport_output[&egui::ViewportId::ROOT]
                        .commands
                        .contains(&egui::ViewportCommand::Close),
                    "New clicks must work after {command:?}"
                );
            }
        }
    }

    #[test]
    fn double_clicking_header_toggles_maximize() {
        for maximized in [false, true] {
            let ctx = egui::Context::default();
            frame(&ctx, vec![], maximized);
            let pos = egui::pos2(230.0, 36.0);
            let mut commands = Vec::new();
            for pressed in [true, false, true, false] {
                let output = frame(
                    &ctx,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    maximized,
                );
                commands.extend(
                    output.viewport_output[&egui::ViewportId::ROOT]
                        .commands
                        .clone(),
                );
            }
            assert!(commands.contains(&egui::ViewportCommand::Maximized(!maximized)));
        }
    }

    #[test]
    fn header_double_click_uses_real_release_times() {
        let ctx = egui::Context::default();
        let pos = egui::pos2(230.0, 36.0);
        let frame = |time, events| {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(300.0, 100.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            };
            raw_input_hook(&ctx, &mut input);
            crate::tests::run_ui(&ctx, input, |ui| {
                header_drag(
                    ui,
                    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 64.0)),
                );
            })
        };
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(0.0, vec![]);
        frame(0.1, vec![egui::Event::PointerMoved(pos), button(true)]);
        // A repaint while the button is still held must not manufacture a click.
        frame(0.12, vec![]);
        frame(0.3, vec![button(false)]);
        frame(0.45, vec![button(true)]);
        let output = frame(0.5, vec![button(false)]);
        assert!(output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::Maximized(true)));
    }

    #[test]
    fn edges_resize_in_all_directions_but_not_when_maximized() {
        use egui::ResizeDirection as D;
        for (pos, direction) in [
            (egui::pos2(2.0, 2.0), D::NorthWest),
            (egui::pos2(298.0, 2.0), D::NorthEast),
            (egui::pos2(2.0, 98.0), D::SouthWest),
            (egui::pos2(298.0, 98.0), D::SouthEast),
            (egui::pos2(150.0, 2.0), D::North),
            (egui::pos2(150.0, 98.0), D::South),
            (egui::pos2(2.0, 50.0), D::West),
            (egui::pos2(298.0, 50.0), D::East),
            // The wider bands and longer corner reach must all be usable.
            (egui::pos2(150.0, 7.0), D::North),
            (egui::pos2(150.0, 93.0), D::South),
            (egui::pos2(7.0, 50.0), D::West),
            (egui::pos2(293.0, 50.0), D::East),
            (egui::pos2(24.0, 5.0), D::NorthWest),
            (egui::pos2(5.0, 24.0), D::NorthWest),
            (egui::pos2(276.0, 5.0), D::NorthEast),
            (egui::pos2(295.0, 24.0), D::NorthEast),
            (egui::pos2(24.0, 95.0), D::SouthWest),
            (egui::pos2(5.0, 76.0), D::SouthWest),
            (egui::pos2(276.0, 95.0), D::SouthEast),
            (egui::pos2(295.0, 76.0), D::SouthEast),
        ] {
            for maximized in [false, true] {
                let ctx = egui::Context::default();
                let frame = |events| {
                    let mut input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(300.0, 100.0),
                        )),
                        events,
                        ..Default::default()
                    };
                    input
                        .viewports
                        .get_mut(&egui::ViewportId::ROOT)
                        .unwrap()
                        .maximized = Some(maximized);
                    crate::tests::run_ui(&ctx, input, |ui| resize_edges(ui.ctx()))
                };
                frame(vec![]);
                frame(vec![]);
                let hover = frame(vec![egui::Event::PointerMoved(pos)]);
                if !maximized {
                    let expected_cursor = match direction {
                        D::NorthWest | D::SouthEast => egui::CursorIcon::ResizeNwSe,
                        D::NorthEast | D::SouthWest => egui::CursorIcon::ResizeNeSw,
                        D::North | D::South => egui::CursorIcon::ResizeVertical,
                        D::West | D::East => egui::CursorIcon::ResizeHorizontal,
                    };
                    assert_eq!(
                        hover.platform_output.cursor_icon, expected_cursor,
                        "Cursor at {pos:?}"
                    );
                }
                let output = frame(vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }]);
                assert_eq!(
                    output.viewport_output[&egui::ViewportId::ROOT]
                        .commands
                        .contains(&egui::ViewportCommand::BeginResize(direction)),
                    !maximized,
                    "Resize {direction:?}, maximized={maximized}"
                );
            }
        }
    }
}
