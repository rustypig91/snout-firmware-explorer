use super::{egui, update, Explorer, Wake, RESTART_PATH};
use crossbeam_channel::{Receiver, TryRecvError};

pub struct Updates {
    pub check_on_startup: bool,
    pub skipped_version: Option<String>,
    check: Option<Receiver<update::CheckResult>>,
    install: Option<Receiver<update::InstallEvent>>,
    manual: bool,
    notice: Option<update::Notice>,
    progress: Option<f32>,
    status: Option<String>,
}
impl Default for Updates {
    fn default() -> Self {
        Self {
            check_on_startup: true,
            skipped_version: None,
            check: None,
            install: None,
            manual: false,
            notice: None,
            progress: None,
            status: None,
        }
    }
}
impl Updates {
    pub fn idle(&self) -> bool {
        self.check.is_none() && self.install.is_none()
    }
}
fn wake(ctx: &egui::Context) -> Wake {
    let ctx = ctx.clone();
    Wake::new(move || ctx.request_repaint())
}
impl Explorer {
    pub(super) fn start_update_check(&mut self, ctx: &egui::Context, manual: bool) {
        if !self.updates.idle() {
            return;
        }
        self.updates.manual = manual;
        match update::spawn_check(wake(ctx)) {
            Ok(rx) => self.updates.check = Some(rx),
            Err(e) if manual => self.updates.notice = Some(update::Notice::Failed(e.to_string())),
            Err(_) => {}
        }
    }
    fn update_failed(&mut self, message: String) {
        self.updates.install = None;
        self.updates.progress = None;
        self.updates.status = Some(format!("Update failed: {message}"));
    }
    pub(super) fn poll_updates(&mut self, ctx: &egui::Context) {
        let checked = match self.updates.check.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(result)) => Some(result),
            Some(Err(TryRecvError::Disconnected)) => {
                Some(Err("The update checker stopped unexpectedly.".into()))
            }
            _ => None,
        };
        if let Some(result) = checked {
            self.updates.check = None;
            self.updates.notice = update::notice_for(
                result,
                env!("CARGO_PKG_VERSION"),
                self.updates.skipped_version.as_deref(),
                self.updates.manual,
            );
            self.updates.status = None;
        }
        loop {
            let event = match self.updates.install.as_ref().map(|rx| rx.try_recv()) {
                Some(Ok(event)) => event,
                Some(Err(TryRecvError::Disconnected)) => {
                    self.update_failed(
                        "The updater stopped unexpectedly. Please try again.".into(),
                    );
                    break;
                }
                _ => break,
            };
            match event {
                update::InstallEvent::Progress { downloaded, total } => {
                    self.updates.progress = Some(downloaded as f32 / total as f32)
                }
                update::InstallEvent::Downloaded(Ok(prepared)) => {
                    if let Err(error) = self.save_preferences() {
                        self.update_failed(format!(
                            "Could not save workspace preferences: {error}"
                        ));
                        break;
                    }
                    match update::spawn_install(prepared, wake(ctx)) {
                        Ok(rx) => {
                            self.updates.install = Some(rx);
                            self.updates.progress = None;
                            self.updates.status = Some("Installing Snout...".into());
                        }
                        Err(e) => self.update_failed(e.to_string()),
                    }
                    break;
                }
                update::InstallEvent::Installed(Ok(outcome)) => {
                    self.updates.install = None;
                    match outcome {
                        update::InstallOutcome::Restart(path) => {
                            *RESTART_PATH.lock().expect("restart path lock") = Some(path)
                        }
                        #[cfg(windows)]
                        update::InstallOutcome::InstallerStarted => {}
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    break;
                }
                update::InstallEvent::Downloaded(Err(e))
                | update::InstallEvent::Installed(Err(e)) => {
                    self.update_failed(e);
                    break;
                }
            }
        }
    }
    pub(super) fn show_updates(&mut self, ctx: &egui::Context) {
        let Some(notice) = self.updates.notice.clone() else {
            return;
        };
        let mut download = None;
        let mut skip = None;
        let mut close = false;
        egui::Window::new("Snout updates")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                if let Some(status) = &self.updates.status {
                    ui.label(status);
                }
                if self.updates.install.is_some() {
                    if let Some(progress) = self.updates.progress {
                        ui.add(egui::ProgressBar::new(progress).show_percentage());
                    } else {
                        ui.spinner();
                    }
                    return;
                }
                match &notice {
                    update::Notice::Available { version, url } => {
                        ui.label(format!(
                            "{version} is available. You are running v{}.",
                            env!("CARGO_PKG_VERSION")
                        ));
                        let packaged = update::is_debian_installation();
                        ui.label(if packaged {
                            update::PACKAGE_UPDATE_MESSAGE
                        } else {
                            "Update will download, install, and restart Snout."
                        });
                        ui.horizontal(|ui| {
                            if !packaged
                                && ui
                                    .add_enabled(
                                        self.updates.idle() && self.receiver.is_none(),
                                        egui::Button::new("Update"),
                                    )
                                    .clicked()
                            {
                                download = Some(version.clone());
                            }
                            if ui.button("Downloads page").clicked() {
                                ctx.open_url(egui::OpenUrl::new_tab(url));
                            }
                            if ui.button("Skip this version").clicked() {
                                skip = Some(version.clone());
                            }
                            if ui.button("Later").clicked() {
                                close = true;
                            }
                        });
                    }
                    update::Notice::UpToDate => {
                        ui.label(format!(
                            "You're running the latest version (v{}).",
                            env!("CARGO_PKG_VERSION")
                        ));
                        close = ui.button("OK").clicked();
                    }
                    update::Notice::Failed(e) => {
                        ui.label(e);
                        close = ui.button("OK").clicked();
                    }
                }
            });
        if let Some(version) = download {
            match update::spawn_download(version, wake(ctx)) {
                Ok(rx) => {
                    self.updates.install = Some(rx);
                    self.updates.progress = Some(0.0);
                    self.updates.status = Some("Downloading Snout...".into());
                }
                Err(e) => self.update_failed(e.to_string()),
            }
        }
        if let Some(version) = skip {
            let previous = self.updates.skipped_version.replace(version);
            if let Err(error) = self.save_preferences() {
                self.updates.skipped_version = previous;
                self.update_failed(format!("Could not save the skipped version: {error}"));
            } else {
                close = true;
            }
        }
        if close {
            self.updates.notice = None;
            self.updates.status = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_startup_checks_stay_quiet_and_manual_checks_report_errors() {
        for manual in [false, true] {
            let mut app = Explorer::default();
            let (tx, rx) = crossbeam_channel::bounded(1);
            tx.send(Err("Offline".into())).unwrap();
            app.updates.check = Some(rx);
            app.updates.manual = manual;
            app.poll_updates(&egui::Context::default());
            assert!(app.updates.idle());
            assert_eq!(app.updates.notice.is_some(), manual);
        }
    }

    #[test]
    fn download_failure_and_disconnected_worker_allow_retry() {
        for event in [
            Some(update::InstallEvent::Downloaded(Err(
                "Checksum mismatch".into()
            ))),
            None,
        ] {
            let mut app = Explorer::default();
            app.updates.notice = Some(update::Notice::Available {
                version: "v1.0.0".into(),
                url: update::RELEASES_PAGE_URL.into(),
            });
            let (tx, rx) = crossbeam_channel::unbounded();
            if let Some(event) = event {
                tx.send(event).unwrap();
            }
            drop(tx);
            app.updates.install = Some(rx);
            app.poll_updates(&egui::Context::default());
            assert!(app.updates.idle());
            assert!(app
                .updates
                .status
                .as_ref()
                .unwrap()
                .starts_with("Update failed:"));
            assert!(matches!(
                app.updates.notice,
                Some(update::Notice::Available { .. })
            ));
        }
    }

    #[test]
    fn update_dialog_renders_notices_and_progress() {
        for notice in [
            update::Notice::UpToDate,
            update::Notice::Failed("Offline".into()),
            update::Notice::Available {
                version: "v1.0.0".into(),
                url: update::RELEASES_PAGE_URL.into(),
            },
        ] {
            let mut app = Explorer::default();
            app.updates.notice = Some(notice);
            let ctx = egui::Context::default();
            let output = ctx.run(egui::RawInput::default(), |ctx| app.show_updates(ctx));
            assert!(!output.shapes.is_empty());
            let (_tx, rx) = crossbeam_channel::unbounded();
            app.updates.install = Some(rx);
            app.updates.progress = Some(0.5);
            let output = ctx.run(egui::RawInput::default(), |ctx| app.show_updates(ctx));
            assert!(!output.shapes.is_empty());
        }
    }
}
