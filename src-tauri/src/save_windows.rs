use super::SaveError;
use std::{fs, os::windows::ffi::OsStrExt, path::Path};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_UNABLE_TO_MOVE_REPLACEMENT_2},
    Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW, ReplaceFileW},
};

pub(super) fn destination_exists(path: &Path) -> Result<bool, SaveError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(SaveError::other("save destination is not a regular file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn wide(path: &Path) -> Result<Vec<u16>, SaveError> {
    let absolute = std::path::absolute(path)?;
    let encoded: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    if encoded.contains(&0) {
        return Err(SaveError::new(
            std::io::ErrorKind::InvalidInput,
            "path contains NUL",
        ));
    }
    let mut extended: Vec<u16> = if encoded.starts_with(&[92, 92, 63, 92]) {
        encoded
    } else if encoded.starts_with(&[92, 92]) {
        "\\\\?\\UNC\\"
            .encode_utf16()
            .chain(encoded[2..].iter().copied())
            .collect()
    } else {
        "\\\\?\\".encode_utf16().chain(encoded).collect()
    };
    extended.push(0);
    Ok(extended)
}

trait CommitOps {
    fn replace(&mut self, destination: &Path, temp: &Path, backup: &Path) -> Result<(), SaveError>;
    fn move_new(&mut self, source: &Path, destination: &Path) -> Result<(), SaveError>;
    fn remove_backup(&mut self, backup: &Path) -> Result<(), SaveError>;
}

struct Windows;

impl CommitOps for Windows {
    fn replace(&mut self, destination: &Path, temp: &Path, backup: &Path) -> Result<(), SaveError> {
        let destination = wide(destination)?;
        let temp = wide(temp)?;
        let backup = wide(backup)?;
        // All three UTF-16 buffers are NUL-terminated and live for this call.
        let result = unsafe {
            ReplaceFileW(
                destination.as_ptr(),
                temp.as_ptr(),
                backup.as_ptr(),
                0,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if result == 0 {
            Err(SaveError::last_os_error())
        } else {
            Ok(())
        }
    }

    fn move_new(&mut self, source: &Path, destination: &Path) -> Result<(), SaveError> {
        let source = wide(source)?;
        let destination = wide(destination)?;
        // No replace/copy flags: a destination that appears concurrently must survive.
        let result = unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        };
        if result == 0 {
            Err(SaveError::last_os_error())
        } else {
            Ok(())
        }
    }

    fn remove_backup(&mut self, backup: &Path) -> Result<(), SaveError> {
        fs::remove_file(backup)
    }
}

pub(super) fn commit(temp: &Path, destination: &Path, existed: bool) -> Result<(), SaveError> {
    let backup = destination.with_file_name(format!(".{}.bak", uuid::Uuid::new_v4()));
    commit_with(temp, destination, existed, &backup, &mut Windows)
}

fn commit_with(
    temp: &Path,
    destination: &Path,
    existed: bool,
    backup: &Path,
    ops: &mut impl CommitOps,
) -> Result<(), SaveError> {
    if !existed {
        return ops.move_new(temp, destination);
    }
    match ops.replace(destination, temp, backup) {
        Ok(()) => {
            // Replacement is already committed; cleanup failure cannot undo success.
            let _ = ops.remove_backup(backup);
            Ok(())
        }
        Err(error) if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32) => {
            ops.move_new(temp, destination)
        }
        Err(error) => {
            if error.raw_os_error() == Some(ERROR_UNABLE_TO_MOVE_REPLACEMENT_2 as i32)
                && backup.try_exists().unwrap_or(false)
            {
                // 1177 can leave the old file at backup. Never replace a racing target.
                let _ = ops.move_new(backup, destination);
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::windows_tests::{dacl, protect};
    use super::*;

    struct Probe {
        error: Option<i32>,
        racer: bool,
        cleanup_error: bool,
        moves: usize,
        real: Windows,
    }

    impl CommitOps for Probe {
        fn replace(
            &mut self,
            destination: &Path,
            temp: &Path,
            backup: &Path,
        ) -> Result<(), SaveError> {
            match self.error {
                Some(1177) => {
                    fs::rename(destination, backup)?;
                    if self.racer {
                        fs::write(destination, "racer")?;
                    }
                    Err(SaveError::from_raw_os_error(1177))
                }
                Some(2) => {
                    fs::remove_file(destination)?;
                    if self.racer {
                        fs::write(destination, "racer")?;
                    }
                    Err(SaveError::from_raw_os_error(2))
                }
                Some(error) => Err(SaveError::from_raw_os_error(error)),
                None => self.real.replace(destination, temp, backup),
            }
        }
        fn move_new(&mut self, source: &Path, destination: &Path) -> Result<(), SaveError> {
            self.moves += 1;
            self.real.move_new(source, destination)
        }
        fn remove_backup(&mut self, backup: &Path) -> Result<(), SaveError> {
            if self.cleanup_error {
                Err(SaveError::from_raw_os_error(5))
            } else {
                self.real.remove_backup(backup)
            }
        }
    }

    #[test]
    fn documented_partial_failure_restores_only_without_replacing_a_racer() {
        for racer in [false, true] {
            let dir = super::super::tests::temp_dir("1177");
            let destination = dir.join("plan.md");
            let temp = dir.join("temp");
            let backup = dir.join("backup");
            fs::write(&destination, "old").unwrap();
            fs::write(&temp, "new").unwrap();
            protect(&destination);
            let before = dacl(&destination);
            let mut ops = Probe {
                error: Some(1177),
                racer,
                cleanup_error: false,
                moves: 0,
                real: Windows,
            };
            assert_eq!(
                commit_with(&temp, &destination, true, &backup, &mut ops)
                    .unwrap_err()
                    .raw_os_error(),
                Some(1177)
            );
            assert_eq!(ops.moves, 1);
            assert_eq!(
                fs::read_to_string(&destination).unwrap(),
                if racer { "racer" } else { "old" }
            );
            if racer {
                assert_eq!(fs::read_to_string(&backup).unwrap(), "old");
                assert!(dacl(&backup) == before);
                assert!(!dacl(&destination).1);
            } else {
                assert!(!backup.exists());
                assert!(dacl(&destination) == before);
            }
            assert_eq!(fs::read_to_string(&temp).unwrap(), "new");
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn disappeared_target_retry_never_replaces_a_racer() {
        for racer in [false, true] {
            let dir = super::super::tests::temp_dir("vanish-retry");
            let destination = dir.join("plan.md");
            let temp = dir.join("temp");
            let backup = dir.join("backup");
            fs::write(&destination, "old").unwrap();
            fs::write(&temp, "new").unwrap();
            let mut ops = Probe {
                error: Some(2),
                racer,
                cleanup_error: false,
                moves: 0,
                real: Windows,
            };
            let result = commit_with(&temp, &destination, true, &backup, &mut ops);
            assert_eq!(result.is_err(), racer);
            assert_eq!(ops.moves, 1);
            assert_eq!(
                fs::read_to_string(&destination).unwrap(),
                if racer { "racer" } else { "new" }
            );
            assert!(!backup.exists());
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn other_errors_never_trigger_restore_or_cleanup() {
        let dir = super::super::tests::temp_dir("no-fallback");
        for code in [5, 1175, 1176] {
            let destination = dir.join("plan.md");
            let temp = dir.join("temp");
            let backup = dir.join("backup");
            fs::write(&destination, "old").unwrap();
            fs::write(&temp, "new").unwrap();
            fs::write(&backup, "unrelated").unwrap();
            let mut ops = Probe {
                error: Some(code),
                racer: false,
                cleanup_error: false,
                moves: 0,
                real: Windows,
            };
            assert_eq!(
                commit_with(&temp, &destination, true, &backup, &mut ops)
                    .unwrap_err()
                    .raw_os_error(),
                Some(code)
            );
            assert_eq!(ops.moves, 0);
            assert_eq!(fs::read_to_string(&destination).unwrap(), "old");
            assert_eq!(fs::read_to_string(&backup).unwrap(), "unrelated");
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cleanup_failure_after_real_replacement_is_still_success() {
        let dir = super::super::tests::temp_dir("cleanup");
        let destination = dir.join("plan.md");
        let temp = dir.join("temp");
        let backup = dir.join("backup");
        fs::write(&destination, "old").unwrap();
        fs::write(&temp, "new").unwrap();
        let mut ops = Probe {
            error: None,
            racer: false,
            cleanup_error: true,
            moves: 0,
            real: Windows,
        };
        commit_with(&temp, &destination, true, &backup, &mut ops).unwrap();
        assert_eq!(fs::read_to_string(&destination).unwrap(), "new");
        assert_eq!(fs::read_to_string(&backup).unwrap(), "old");
        assert!(!temp.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn paths_are_extended_and_reject_nul() {
        assert!(
            wide(Path::new("plan.md"))
                .unwrap()
                .starts_with(&[92, 92, 63, 92])
        );
        assert!(wide(Path::new("bad\0name")).is_err());
    }
}
