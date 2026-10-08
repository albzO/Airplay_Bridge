//! Shared GUI/CLI data directory, independent of the application identifier.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub fn prepare() -> io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    if let Some(root) = portable_root(&exe) {
        let destination = root.join("data");
        fs::create_dir_all(&destination)?;
        return Ok(destination);
    }
    let base = std::env::var_os("APPDATA").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "APPDATA 未设置，无法定位用户数据目录",
        )
    })?;
    prepare_at(Path::new(&base))
}

/// A marker next to the GUI enables portability for both the GUI and tools/ CLI.
/// Resolve from the executable, never from the caller's working directory.
pub fn portable_root(exe: &Path) -> Option<PathBuf> {
    let directory = exe.parent()?;
    if directory.join("portable.flag").is_file() {
        return Some(directory.to_path_buf());
    }
    if directory.file_name()?.eq_ignore_ascii_case("tools") {
        let root = directory.parent()?;
        if root.join("portable.flag").is_file() {
            return Some(root.to_path_buf());
        }
    }
    None
}

fn prepare_at(base: &Path) -> io::Result<PathBuf> {
    // Both fixed names are direct children of the resolved AppData directory.
    // Retain the legacy directory as a backup and never overwrite newer data.
    let base = base.canonicalize()?;
    let destination = base.join("AirPlay Hub");
    let legacy = base.join("com.airplaywin.bridge");
    fs::create_dir_all(&destination)?;
    let marker = destination.join(".legacy-data-migrated");
    if legacy.is_dir() && !marker.exists() {
        copy_missing(&legacy, &destination)?;
        fs::write(marker, b"com.airplaywin.bridge\n")?;
    }
    Ok(destination)
}

fn copy_missing(source: &Path, destination: &Path) -> io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = destination.join(entry.file_name());
        // Do not follow links out of the legacy data tree.
        if kind.is_dir() {
            fs::create_dir_all(&target)?;
            copy_missing(&entry.path(), &target)?;
        } else if kind.is_file() {
            let mut input = fs::File::open(entry.path())?;
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
            {
                Ok(mut output) => {
                    if let Err(error) = io::copy(&mut input, &mut output) {
                        drop(output);
                        let _ = fs::remove_file(&target);
                        return Err(error);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_marker_selects_gui_and_cli_but_not_other_directories() {
        let base = base();
        assert_eq!(portable_root(&base.join("airplay-bridge.exe")), None);
        fs::write(base.join("portable.flag"), b"").unwrap();
        assert_eq!(
            portable_root(&base.join("airplay-bridge.exe")),
            Some(base.clone())
        );
        assert_eq!(
            portable_root(&base.join("tools/homepod-test.exe")),
            Some(base.clone())
        );
        assert_eq!(portable_root(&base.join("other/test.exe")), None);
        assert!(!base.join("data").exists());
        fs::remove_file(base.join("portable.flag")).unwrap();
        fs::remove_dir(base).unwrap();
    }
    fn base() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "airplay-data-dir-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        path.canonicalize().unwrap()
    }
    #[test]
    fn migrates_legacy_settings_and_cli_together_and_is_repeatable() {
        let base = base();
        let old = base.join("com.airplaywin.bridge");
        fs::create_dir_all(old.join("cli")).unwrap();
        fs::write(old.join("settings.json"), b"existing settings").unwrap();
        fs::write(old.join("cli/devices.json"), b"existing devices").unwrap();
        let new = prepare_at(&base).unwrap();
        assert_eq!(new, base.join("AirPlay Hub"));
        assert!(old.exists());
        assert_eq!(
            fs::read(new.join("settings.json")).unwrap(),
            b"existing settings"
        );
        assert_eq!(
            fs::read(new.join("cli/devices.json")).unwrap(),
            b"existing devices"
        );
        assert_eq!(prepare_at(&base).unwrap(), new);
        fs::remove_file(new.join("settings.json")).unwrap();
        fs::remove_file(new.join("cli/devices.json")).unwrap();
        fs::remove_file(new.join(".legacy-data-migrated")).unwrap();
        fs::remove_file(old.join("settings.json")).unwrap();
        fs::remove_file(old.join("cli/devices.json")).unwrap();
        fs::remove_dir(old.join("cli")).unwrap();
        fs::remove_dir(old).unwrap();
        fs::remove_dir(new.join("cli")).unwrap();
        fs::remove_dir(new).unwrap();
        fs::remove_dir(base).unwrap();
    }
    #[test]
    fn existing_new_directory_is_preserved() {
        let base = base();
        let old = base.join("com.airplaywin.bridge");
        let new = base.join("AirPlay Hub");
        fs::create_dir(&old).unwrap();
        fs::create_dir(&new).unwrap();
        fs::write(old.join("settings.json"), b"old").unwrap();
        fs::write(old.join("old.log"), b"history").unwrap();
        fs::write(new.join("settings.json"), b"new").unwrap();
        assert_eq!(prepare_at(&base).unwrap(), new);
        assert_eq!(fs::read(new.join("settings.json")).unwrap(), b"new");
        assert!(old.exists());
        assert_eq!(fs::read(new.join("old.log")).unwrap(), b"history");
        // Once migrated, deleted data must not reappear on subsequent launches.
        fs::remove_file(new.join("old.log")).unwrap();
        prepare_at(&base).unwrap();
        assert!(!new.join("old.log").exists());
        fs::remove_file(old.join("settings.json")).unwrap();
        fs::remove_file(old.join("old.log")).unwrap();
        fs::remove_file(new.join("settings.json")).unwrap();
        fs::remove_file(new.join(".legacy-data-migrated")).unwrap();
        fs::remove_dir(old).unwrap();
        fs::remove_dir(new).unwrap();
        fs::remove_dir(base).unwrap();
    }
}
