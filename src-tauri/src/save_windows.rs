use super::SaveError;
use std::{
    fs,
    fs::File,
    os::windows::{ffi::OsStrExt, io::FromRawHandle},
    path::Path,
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_NETWORK_ACCESS_DENIED,
        ERROR_PRIVILEGE_NOT_HELD, ERROR_UNABLE_TO_MOVE_REPLACEMENT_2, GENERIC_WRITE,
        INVALID_HANDLE_VALUE,
    },
    Security::{
        DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, GetFileSecurityW, GetLengthSid,
        GetSecurityDescriptorControl, GetSecurityDescriptorDacl, GetSecurityDescriptorGroup,
        GetSecurityDescriptorOwner, OWNER_SECURITY_INFORMATION, SE_DACL_PROTECTED,
        SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::{
        CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        REPLACEFILE_IGNORE_ACL_ERRORS, REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW,
    },
};

pub(super) fn destination_exists(path: &Path) -> Result<bool, SaveError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            fs::OpenOptions::new().write(true).open(path)?;
            Ok(true)
        }
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

struct Security(Vec<u32>);

#[derive(PartialEq, Eq)]
struct SecurityIdentity {
    owner: Vec<u8>,
    group: Vec<u8>,
    dacl: Option<Vec<u8>>,
    dacl_present: bool,
    protected: bool,
}

impl Security {
    fn read(path: &Path) -> Result<Self, SaveError> {
        let path = wide(path)?;
        let information =
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let mut needed = 0;
        // The first call obtains the required length; u32 storage aligns the descriptor.
        unsafe {
            GetFileSecurityW(
                path.as_ptr(),
                information,
                std::ptr::null_mut(),
                0,
                &mut needed,
            );
        }
        if needed == 0 {
            return Err(SaveError::last_os_error());
        }
        let mut buffer = vec![0u32; (needed as usize).div_ceil(4)];
        if unsafe {
            GetFileSecurityW(
                path.as_ptr(),
                information,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        } == 0
        {
            return Err(SaveError::last_os_error());
        }
        Ok(Self(buffer))
    }

    fn identity(&self) -> Result<SecurityIdentity, SaveError> {
        let descriptor = self.0.as_ptr().cast_mut().cast();
        let mut owner = std::ptr::null_mut();
        let mut group = std::ptr::null_mut();
        let mut acl = std::ptr::null_mut();
        let mut present = 0;
        let mut defaulted = 0;
        let mut control = 0;
        let mut revision = 0;
        // These pointers refer to validated self-relative data returned by Windows.
        unsafe {
            if GetSecurityDescriptorOwner(descriptor, &mut owner, &mut defaulted) == 0
                || GetSecurityDescriptorGroup(descriptor, &mut group, &mut defaulted) == 0
                || GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted)
                    == 0
                || GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) == 0
            {
                return Err(SaveError::last_os_error());
            }
            if owner.is_null() || group.is_null() {
                return Err(SaveError::new(
                    std::io::ErrorKind::InvalidData,
                    "export security descriptor has no owner or group",
                ));
            }
            Ok(SecurityIdentity {
                owner: std::slice::from_raw_parts(owner.cast(), GetLengthSid(owner) as usize)
                    .to_vec(),
                group: std::slice::from_raw_parts(group.cast(), GetLengthSid(group) as usize)
                    .to_vec(),
                dacl: if present != 0 && !acl.is_null() {
                    Some(std::slice::from_raw_parts(acl.cast(), (*acl).AclSize as usize).to_vec())
                } else {
                    None
                },
                dacl_present: present != 0,
                protected: control & SE_DACL_PROTECTED != 0,
            })
        }
    }
}

pub(super) fn create_staged(temp: &Path, destination: &Path) -> Result<File, SaveError> {
    let mut security = if destination_exists(destination)? {
        Some(Security::read(destination)?)
    } else {
        None
    };
    let expected = security.as_ref().map(Security::identity).transpose()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security
            .as_mut()
            .map_or(std::ptr::null_mut(), |s| s.0.as_mut_ptr().cast()),
        bInheritHandle: 0,
    };
    let name = wide(temp)?;
    // CREATE_NEW attaches the destination security before the stage can contain data.
    // A foreign owner/group that cannot be assigned fails closed, without privileges.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(SaveError::last_os_error());
    }
    // The unique successful handle is transferred to File exactly once.
    let file = unsafe { File::from_raw_handle(handle) };
    let verified = (|| {
        if let Some(expected) = expected {
            if Security::read(temp)?.identity()? != expected {
                return Err(SaveError::new(
                    std::io::ErrorKind::PermissionDenied,
                    "staged export security differs from destination; no content written",
                ));
            }
        }
        Ok(())
    })();
    if let Err(error) = verified {
        drop(file);
        let _ = fs::remove_file(temp);
        return Err(error);
    }
    Ok(file)
}

trait CommitOps {
    fn replace(
        &mut self,
        destination: &Path,
        temp: &Path,
        backup: &Path,
        flags: u32,
    ) -> Result<(), SaveError>;
    fn move_new(&mut self, source: &Path, destination: &Path) -> Result<(), SaveError>;
    fn move_replace(
        &mut self,
        source: &Path,
        destination: &Path,
        flags: u32,
    ) -> Result<(), SaveError>;
    fn remove_backup(&mut self, backup: &Path) -> Result<(), SaveError>;
}

struct Windows;

impl CommitOps for Windows {
    fn replace(
        &mut self,
        destination: &Path,
        temp: &Path,
        backup: &Path,
        flags: u32,
    ) -> Result<(), SaveError> {
        destination_exists(destination)?;
        let destination = wide(destination)?;
        let temp = wide(temp)?;
        let backup = wide(backup)?;
        // All three UTF-16 buffers are NUL-terminated and live for this call.
        let result = unsafe {
            ReplaceFileW(
                destination.as_ptr(),
                temp.as_ptr(),
                backup.as_ptr(),
                flags,
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
        move_file(source, destination, MOVEFILE_WRITE_THROUGH)
    }

    fn move_replace(
        &mut self,
        source: &Path,
        destination: &Path,
        flags: u32,
    ) -> Result<(), SaveError> {
        destination_exists(destination)?;
        move_file(source, destination, flags)
    }

    fn remove_backup(&mut self, backup: &Path) -> Result<(), SaveError> {
        fs::remove_file(backup)
    }
}

fn move_file(source: &Path, destination: &Path, flags: u32) -> Result<(), SaveError> {
    let source = wide(source)?;
    let destination = wide(destination)?;
    // Both UTF-16 buffers are terminated and remain live throughout the call.
    let result = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), flags) };
    if result == 0 {
        Err(SaveError::last_os_error())
    } else {
        Ok(())
    }
}

pub(super) fn commit(temp: &Path, destination: &Path, existed: bool) -> Result<(), SaveError> {
    let backup = destination.with_file_name(format!(".{}.bak", uuid::Uuid::new_v4()));
    commit_with(temp, destination, existed, &backup, &mut Windows)
}

fn committed(backup: &Path, ops: &mut impl CommitOps) -> Result<(), SaveError> {
    // The destination already contains the complete export. Cleanup cannot undo success.
    let _ = ops.remove_backup(backup);
    Ok(())
}

fn permission_error(error: &SaveError) -> bool {
    matches!(error.raw_os_error(), Some(code) if code == ERROR_ACCESS_DENIED as i32
        || code == ERROR_NETWORK_ACCESS_DENIED as i32 || code == ERROR_PRIVILEGE_NOT_HELD as i32)
}

fn restore_partial(
    error: SaveError,
    destination: &Path,
    backup: &Path,
    ops: &mut impl CommitOps,
) -> SaveError {
    if error.raw_os_error() == Some(ERROR_UNABLE_TO_MOVE_REPLACEMENT_2 as i32)
        && backup.try_exists().unwrap_or(false)
        && let Err(restore_error) = ops.move_new(backup, destination)
    {
        return SaveError::new(
            error.kind(),
            format!(
                "{error}; could not restore previous export ({restore_error}); recovery backup: {}",
                backup.display()
            ),
        );
    }
    error
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
    let first = match ops.replace(destination, temp, backup, 0) {
        Ok(()) => return committed(backup, ops),
        Err(error) if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32) => {
            return ops.move_new(temp, destination);
        }
        Err(error) if permission_error(&error) => error,
        Err(error) => return Err(restore_partial(error, destination, backup, ops)),
    };
    let retry = match ops.replace(
        destination,
        temp,
        backup,
        REPLACEFILE_IGNORE_MERGE_ERRORS | REPLACEFILE_IGNORE_ACL_ERRORS,
    ) {
        Ok(()) => return committed(backup, ops),
        Err(error) => error,
    };
    match ops.move_replace(
        temp,
        destination,
        MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
    ) {
        Ok(()) => committed(backup, ops),
        Err(final_error) => {
            let recovery = restore_partial(retry, destination, backup, ops);
            Err(SaveError::new(
                final_error.kind(),
                format!(
                    "Export replacement failed: strict replace ({first}); relaxed replace ({recovery}); final move ({final_error})"
                ),
            ))
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
            flags: u32,
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
                None => self.real.replace(destination, temp, backup, flags),
            }
        }
        fn move_new(&mut self, source: &Path, destination: &Path) -> Result<(), SaveError> {
            self.moves += 1;
            self.real.move_new(source, destination)
        }
        fn move_replace(
            &mut self,
            source: &Path,
            destination: &Path,
            flags: u32,
        ) -> Result<(), SaveError> {
            self.real.move_replace(source, destination, flags)
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
            let error = commit_with(&temp, &destination, true, &backup, &mut ops).unwrap_err();
            if racer {
                assert!(error.to_string().contains("recovery backup:"));
            } else {
                assert_eq!(error.raw_os_error(), Some(1177));
            }
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
        for code in [32, 1175, 1176] {
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
    struct ScriptedOps {
        errors: std::collections::VecDeque<i32>,
        move_error: Option<i32>,
        restore_error: bool,
        replace_flags: Vec<u32>,
        move_flags: Vec<u32>,
        real: Windows,
    }

    impl ScriptedOps {
        fn new(errors: &[i32], move_error: Option<i32>, restore_error: bool) -> Self {
            Self {
                errors: errors.iter().copied().collect(),
                move_error,
                restore_error,
                replace_flags: vec![],
                move_flags: vec![],
                real: Windows,
            }
        }
    }

    impl CommitOps for ScriptedOps {
        fn replace(
            &mut self,
            destination: &Path,
            temp: &Path,
            backup: &Path,
            flags: u32,
        ) -> Result<(), SaveError> {
            self.replace_flags.push(flags);
            match self.errors.pop_front().expect("unexpected replacement") {
                0 => self.real.replace(destination, temp, backup, flags),
                1177 => {
                    fs::rename(destination, backup)?;
                    Err(SaveError::from_raw_os_error(1177))
                }
                code => Err(SaveError::from_raw_os_error(code)),
            }
        }
        fn move_new(&mut self, source: &Path, destination: &Path) -> Result<(), SaveError> {
            if self.restore_error {
                Err(SaveError::from_raw_os_error(5))
            } else {
                self.real.move_new(source, destination)
            }
        }
        fn move_replace(
            &mut self,
            source: &Path,
            destination: &Path,
            flags: u32,
        ) -> Result<(), SaveError> {
            self.move_flags.push(flags);
            if let Some(code) = self.move_error {
                Err(SaveError::from_raw_os_error(code))
            } else {
                self.real.move_replace(source, destination, flags)
            }
        }
        fn remove_backup(&mut self, backup: &Path) -> Result<(), SaveError> {
            self.real.remove_backup(backup)
        }
    }

    fn export_with_script(
        destination: &Path,
        backup: &Path,
        ops: &mut ScriptedOps,
    ) -> Result<super::super::SaveResponse, SaveError> {
        super::super::write_export_transaction(
            destination,
            "complete replacement",
            |file, bytes| std::io::Write::write_all(file, bytes),
            |temp| commit_with(temp, destination, true, backup, ops),
        )
    }

    #[test]
    fn permission_errors_retry_with_exact_relaxed_flags() {
        for code in [5, 65, 1314] {
            let dir = super::super::tests::temp_dir("relaxed-permission");
            let destination = dir.join("plan.md");
            let backup = dir.join("backup");
            fs::write(&destination, "previous export").unwrap();
            let mut ops = ScriptedOps::new(&[code, 0], Some(5), false);
            export_with_script(&destination, &backup, &mut ops).unwrap();
            assert_eq!(ops.replace_flags, [0, 0x6]);
            assert!(ops.move_flags.is_empty());
            assert_eq!(
                fs::read_to_string(&destination).unwrap(),
                "complete replacement"
            );
            assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn failed_relaxed_replace_uses_exact_atomic_move_flags() {
        let dir = super::super::tests::temp_dir("final-move");
        let destination = dir.join("plan.md");
        let backup = dir.join("backup");
        fs::write(&destination, "previous export").unwrap();
        let mut ops = ScriptedOps::new(&[5, 5], None, false);
        export_with_script(&destination, &backup, &mut ops).unwrap();
        assert_eq!(ops.replace_flags, [0, 0x6]);
        assert_eq!(ops.move_flags, [0x9]);
        assert_eq!(
            fs::read_to_string(&destination).unwrap(),
            "complete replacement"
        );
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn all_fallbacks_failing_preserve_old_bytes_and_acl_and_clean_the_temp() {
        for second_error in [5, 32, 1175, 1176] {
            let dir = super::super::tests::temp_dir("all-failed");
            let destination = dir.join("plan.md");
            let backup = dir.join("backup");
            fs::write(&destination, "previous export").unwrap();
            protect(&destination);
            let before = dacl(&destination);
            let mut ops = ScriptedOps::new(&[5, second_error], Some(5), false);
            let error = export_with_script(&destination, &backup, &mut ops).unwrap_err();
            assert!(error.to_string().contains("strict replace"));
            assert!(error.to_string().contains("relaxed replace"));
            assert!(error.to_string().contains("final move"));
            assert_eq!(ops.replace_flags, [0, 0x6]);
            assert_eq!(ops.move_flags, [0x9]);
            assert_eq!(fs::read_to_string(&destination).unwrap(), "previous export");
            assert!(dacl(&destination) == before);
            assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn partial_relaxed_failure_restores_old_file_if_final_move_fails() {
        for restore_error in [false, true] {
            let dir = super::super::tests::temp_dir("partial-relaxed");
            let destination = dir.join("plan.md");
            let backup = dir.join("backup");
            fs::write(&destination, "previous export").unwrap();
            protect(&destination);
            let before = dacl(&destination);
            let mut ops = ScriptedOps::new(&[5, 1177], Some(5), restore_error);
            let error = export_with_script(&destination, &backup, &mut ops).unwrap_err();
            assert_eq!(ops.replace_flags, [0, 0x6]);
            assert_eq!(ops.move_flags, [0x9]);
            let preserved = if restore_error { &backup } else { &destination };
            assert_eq!(fs::read_to_string(preserved).unwrap(), "previous export");
            assert!(dacl(preserved) == before);
            assert_eq!(
                fs::read_dir(&dir).unwrap().count(),
                1,
                "partial temp must be removed"
            );
            if restore_error {
                assert!(error.to_string().contains(&backup.display().to_string()));
                assert!(!destination.exists());
            } else {
                assert!(!backup.exists());
            }
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn partial_relaxed_failure_can_complete_via_final_move() {
        let dir = super::super::tests::temp_dir("partial-complete");
        let destination = dir.join("plan.md");
        let backup = dir.join("backup");
        fs::write(&destination, "previous export").unwrap();
        let mut ops = ScriptedOps::new(&[5, 1177], None, false);
        export_with_script(&destination, &backup, &mut ops).unwrap();
        assert_eq!(ops.replace_flags, [0, 0x6]);
        assert_eq!(ops.move_flags, [0x9]);
        assert_eq!(
            fs::read_to_string(&destination).unwrap(),
            "complete replacement"
        );
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn nonpermission_first_error_does_not_relax_or_replace_move() {
        let dir = super::super::tests::temp_dir("no-relax");
        let destination = dir.join("plan.md");
        let backup = dir.join("backup");
        fs::write(&destination, "previous export").unwrap();
        let mut ops = ScriptedOps::new(&[32, 0], None, false);
        assert_eq!(
            export_with_script(&destination, &backup, &mut ops)
                .unwrap_err()
                .raw_os_error(),
            Some(32)
        );
        assert_eq!(ops.replace_flags, [0]);
        assert!(ops.move_flags.is_empty());
        assert_eq!(fs::read_to_string(&destination).unwrap(), "previous export");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn real_final_move_cannot_replace_a_nonwritable_destination() {
        let dir = super::super::tests::temp_dir("move-write-access");
        let destination = dir.join("plan.md");
        let temp = dir.join("stage");
        fs::write(&destination, "old").unwrap();
        fs::write(&temp, "new").unwrap();
        super::super::windows_tests::set_dacl(&destination, "D:P(A;;GRSD;;;OW)");
        let before = super::super::windows_tests::dacl(&destination);
        let error = Windows
            .move_replace(
                &temp,
                &destination,
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read_to_string(&destination).unwrap(), "old");
        assert_eq!(super::super::windows_tests::dacl(&destination), before);
        assert!(temp.exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
