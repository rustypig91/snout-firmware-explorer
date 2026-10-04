use std::{ffi::OsString, path::PathBuf};

#[derive(Debug, Default)]
pub struct Startup {
    pub folder: Option<PathBuf>,
    pub elf: Option<PathBuf>,
    pub no_update_check: bool,
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Option<Startup>, String> {
    let mut result = Startup::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--help" | "-h") => {
                println!("Usage: firmware-gui [BUILD_FOLDER] [--elf FILE] [--no-update-check]\nAn ELF path can also be passed directly. Relative --elf paths are resolved inside BUILD_FOLDER.");
                return Ok(None);
            }
            Some("--elf") => {
                if result.elf.is_some() {
                    return Err("--elf may only be supplied once.".into());
                }
                result.elf = Some(
                    args.next()
                        .filter(|s| !s.is_empty())
                        .map(PathBuf::from)
                        .ok_or("--elf requires a file path.")?,
                );
            }
            Some("--no-update-check") => result.no_update_check = true,
            Some(s) if s.starts_with('-') => return Err(format!("Unknown argument: {s}")),
            _ => {
                if result.folder.is_some() {
                    return Err("Only one build folder or ELF path may be supplied.".into());
                }
                result.folder = Some(arg.into());
            }
        }
    }
    if result.elf.is_none() && result.folder.as_ref().is_some_and(|p| p.is_file()) {
        result.elf = result.folder.take();
    }
    if let Some(elf) = result.elf.take() {
        let elf = if elf.is_relative() {
            result
                .folder
                .as_ref()
                .map_or_else(|| elf.clone(), |folder| folder.join(&elf))
        } else {
            elf
        };
        let elf = elf
            .canonicalize()
            .map_err(|e| format!("Could not open {}: {e}", elf.display()))?;
        if !elf.is_file() {
            return Err("--elf must refer to a firmware file.".into());
        }
        if result.folder.is_none() {
            result.folder = elf.parent().map(PathBuf::from);
        }
        result.elf = Some(elf);
    }
    if let Some(folder) = &mut result.folder {
        *folder = folder
            .canonicalize()
            .map_err(|e| format!("Could not open {}: {e}", folder.display()))?;
        if !folder.is_dir() {
            return Err("The build folder must be a directory.".into());
        }
        if result
            .elf
            .as_ref()
            .is_some_and(|elf| !elf.starts_with(&*folder))
        {
            return Err("The selected ELF must be inside the build folder.".into());
        }
    }
    Ok(Some(result))
}

impl super::Explorer {
    pub(super) fn open_startup(&mut self, startup: &Startup) {
        if let Some(folder) = &startup.folder {
            self.pending_restore = startup.elf.clone().map(|path| {
                let saved = self
                    .build_settings
                    .get(folder)
                    .and_then(|settings| settings.layouts.get(&path));
                (
                    path,
                    saved.map(|layout| layout.options.clone()),
                    saved
                        .map(|layout| layout.source.clone())
                        .unwrap_or_default(),
                )
            });
            self.view = super::View::Overview;
            self.scan_build(folder.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .canonicalize()
            .unwrap()
    }
    #[test]
    fn selects_firmware_relative_to_folder_or_directly() {
        let folder = fixtures();
        let elf = folder.join("cortex-m.elf");
        let explicit = parse([
            folder.clone().into_os_string(),
            "--elf".into(),
            "cortex-m.elf".into(),
            "--no-update-check".into(),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(explicit.elf, Some(elf.clone()));
        assert!(explicit.no_update_check);
        for args in [
            vec![elf.clone().into_os_string()],
            vec!["--elf".into(), elf.clone().into_os_string()],
        ] {
            let direct = parse(args).unwrap().unwrap();
            assert_eq!(direct.folder, Some(folder.clone()));
            assert_eq!(direct.elf, Some(elf.clone()));
        }
    }
    #[test]
    fn rejects_missing_unknown_and_outside_paths() {
        for args in [
            vec!["--elf".into()],
            vec!["--unknown".into()],
            vec!["--elf".into(), "does-not-exist.elf".into()],
        ] {
            assert!(parse(args).is_err());
        }
        assert!(parse([
            fixtures().join("src").into_os_string(),
            "--elf".into(),
            fixtures().join("cortex-m.elf").into_os_string()
        ])
        .is_err());
    }
}
