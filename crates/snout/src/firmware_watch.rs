//! Metadata-only monitoring of the ELF represented by the current report.
use std::{
    path::PathBuf,
    sync::mpsc,
    time::{Duration, SystemTime},
};

const CHECK_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    length: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

fn stamp(path: &std::path::Path) -> Option<Stamp> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(Stamp {
        length: metadata.len(),
        modified: metadata.modified().ok(),
        created: metadata.created().ok(),
        #[cfg(unix)]
        identity: {
            use std::os::unix::fs::MetadataExt;
            (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            )
        },
    })
}

#[derive(Clone)]
pub(super) struct Observation {
    path: PathBuf,
    stamp: Option<Stamp>,
}
impl Observation {
    // Capture before analysis, on its worker, so edits during a load aren't
    // silently acknowledged as belonging to the displayed report.
    pub(super) fn capture(path: PathBuf) -> Self {
        let stamp = stamp(&path);
        Self { path, stamp }
    }
    fn changed(&self) -> Option<bool> {
        // An unavailable file supplies no evidence of a changed firmware.
        stamp(&self.path).map(|current| self.stamp.as_ref() != Some(&current))
    }
}

struct Worker {
    commands: mpsc::Sender<(u64, Option<Observation>)>,
    changes: mpsc::Receiver<(u64, bool)>,
}
impl Worker {
    fn start(ctx: eframe::egui::Context) -> Self {
        Self::with_interval(ctx, CHECK_INTERVAL)
    }
    fn with_interval(ctx: eframe::egui::Context, interval: Duration) -> Self {
        let (commands, requests) = mpsc::channel::<(u64, Option<Observation>)>();
        let (notifications, changes) = mpsc::channel();
        std::thread::spawn(move || {
            let mut current: Option<(u64, Observation, bool)> = None;
            let mut last_visible = None;
            loop {
                // Continue checking availability after a change so a missing
                // ELF never keeps the badge visible. No selected ELF means no
                // polling; dropping the sender stops the worker without a join.
                let request = if current.is_some() {
                    requests.recv_timeout(interval)
                } else {
                    requests
                        .recv()
                        .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
                };
                match request {
                    Ok((generation, observation)) => {
                        current = observation.map(|observation| (generation, observation, false));
                        last_visible = None;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                if let Some((generation, observation, latched)) = &mut current {
                    let change = observation.changed();
                    *latched |= change.unwrap_or(false);
                    let visible = change.is_some() && *latched;
                    if last_visible != Some(visible) {
                        let _ = notifications.send((*generation, visible));
                        ctx.request_repaint();
                        last_visible = Some(visible);
                    }
                }
            }
        });
        Self { commands, changes }
    }
}

#[derive(Default)]
pub(super) struct FirmwareWatch {
    observation: Option<Observation>,
    generation: u64,
    changed: bool,
    worker: Option<Worker>,
}
impl FirmwareWatch {
    pub(super) fn set(&mut self, observation: Option<Observation>) {
        self.generation = self.generation.wrapping_add(1);
        self.changed = false;
        self.observation = observation;
        if let Some(worker) = &self.worker {
            let _ = worker
                .commands
                .send((self.generation, self.observation.clone()));
        }
    }
    pub(super) fn poll(&mut self, ctx: &eframe::egui::Context) {
        if self.worker.is_none() && self.observation.is_some() {
            let worker = Worker::start(ctx.clone());
            let _ = worker
                .commands
                .send((self.generation, self.observation.clone()));
            self.worker = Some(worker);
        }
        if let Some(worker) = &self.worker {
            for (generation, visible) in worker.changes.try_iter() {
                if generation == self.generation {
                    self.changed = visible;
                }
            }
        }
    }
    pub(super) fn changed(&self) -> bool {
        self.changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_detects_edits_and_replacement_but_ignores_missing_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("zephyr.elf");
        std::fs::write(&path, b"old ELF").unwrap();
        let baseline = Observation::capture(path.clone());
        assert_eq!(baseline.changed(), Some(false));
        std::fs::write(&path, b"new ELF").unwrap();
        let later = baseline.stamp.as_ref().unwrap().modified.unwrap() + Duration::from_secs(1);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert_eq!(baseline.changed(), Some(true));
        let baseline = Observation::capture(path.clone());
        let replacement = directory.path().join("new.elf");
        std::fs::write(&replacement, b"another ELF").unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::rename(replacement, &path).unwrap();
        assert_eq!(baseline.changed(), Some(true));
        let baseline = Observation::capture(path.clone());
        std::fs::remove_file(&path).unwrap();
        assert_eq!(baseline.changed(), None);
        let missing = Observation::capture(path.clone());
        std::fs::write(path, b"returned ELF").unwrap();
        assert_eq!(missing.changed(), Some(true));
    }

    #[test]
    fn worker_reports_changes_and_stops_when_owner_disconnects() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("zephyr.elf");
        std::fs::write(&path, b"old").unwrap();
        let observation = Observation::capture(path.clone());
        std::fs::write(path, b"new firmware").unwrap();
        let worker = Worker::start(eframe::egui::Context::default());
        worker.commands.send((7, Some(observation))).unwrap();
        assert_eq!(
            worker.changes.recv_timeout(Duration::from_secs(2)).unwrap(),
            (7, true)
        );
        // Disconnecting commands also interrupts the metadata polling wait.
        drop(worker.commands);
        assert!(matches!(
            worker.changes.recv_timeout(Duration::from_secs(2)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn missing_files_hide_the_badge_and_reappearance_is_still_monitored() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("zephyr.elf");
        std::fs::write(&path, b"old firmware").unwrap();
        let observation = Observation::capture(path.clone());
        std::fs::remove_file(&path).unwrap();
        let worker =
            Worker::with_interval(eframe::egui::Context::default(), Duration::from_millis(10));
        worker.commands.send((1, Some(observation))).unwrap();
        let receive = || worker.changes.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            receive(),
            (1, false),
            "Deletion must not mark firmware changed"
        );
        assert!(
            matches!(
                worker.changes.recv_timeout(Duration::from_millis(40)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "Missing files must not repeatedly notify"
        );
        std::fs::write(&path, b"new firmware, rebuilt").unwrap();
        assert_eq!(receive(), (1, true));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            receive(),
            (1, false),
            "An existing badge must hide when the file disappears"
        );
        std::fs::write(path, b"new firmware, rebuilt again").unwrap();
        assert_eq!(receive(), (1, true));
    }

    #[test]
    fn old_notifications_cannot_mark_a_new_report_changed() {
        let (commands, _requests) = mpsc::channel();
        let (notifications, changes) = mpsc::channel();
        let mut watch = FirmwareWatch {
            worker: Some(Worker { commands, changes }),
            ..Default::default()
        };
        notifications.send((watch.generation, true)).unwrap();
        watch.set(None);
        watch.poll(&eframe::egui::Context::default());
        assert!(!watch.changed());
        notifications.send((watch.generation, true)).unwrap();
        watch.poll(&eframe::egui::Context::default());
        assert!(watch.changed());
        watch.set(None);
        assert!(!watch.changed());
    }
}
