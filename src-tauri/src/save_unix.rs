use std::{
    fs,
    fs::File,
    io,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};

pub(crate) fn preflight(destination: &Path) -> io::Result<()> {
    // Ask the OS using the same write access as File::create, without truncation.
    // Mode bits alone would miss POSIX ACLs and effective-credential permissions.
    match fs::metadata(destination) {
        Ok(metadata) if !metadata.is_file() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "save destination is not a regular file",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    match fs::OpenOptions::new().write(true).open(destination) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(crate) fn create_staged(temp: &Path, destination: &Path) -> io::Result<File> {
    preflight(destination)?;
    let mode = match fs::symlink_metadata(destination) {
        Ok(_) => 0o600,
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0o666,
        Err(error) => return Err(error),
    };
    // Existing exports start private even during the empty-stage preparation window.
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(temp)
}

fn require_matching_ownership(
    destination_uid: u32,
    destination_gid: u32,
    staged_uid: u32,
    staged_gid: u32,
) -> io::Result<()> {
    if (destination_uid, destination_gid) != (staged_uid, staged_gid) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "existing export owner or group differs from the staged file; overwrite refused",
        ));
    }
    Ok(())
}

fn existing_permissions(
    destination: &Path,
    staged: &fs::Metadata,
) -> io::Result<Option<fs::Permissions>> {
    match fs::symlink_metadata(destination) {
        Ok(existing) if existing.file_type().is_file() => {
            require_matching_ownership(existing.uid(), existing.gid(), staged.uid(), staged.gid())?;
            Ok(Some(existing.permissions()))
        }
        // Preserve rename's link-entry semantics; never adopt the link target's metadata.
        Ok(_) => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) fn prepare_staged(file: &File, destination: &Path) -> io::Result<()> {
    preflight(destination)?;
    if let Some(permissions) = existing_permissions(destination, &file.metadata()?)? {
        // Restrict the empty stage before private content is written to it.
        file.set_permissions(permissions)?;
    } else if fs::symlink_metadata(destination).is_ok_and(|m| m.file_type().is_symlink()) {
        // Rename replaces the link entry. Keep its new content private without
        // adopting the target's ownership or changing the target itself.
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub(crate) fn commit(temp: &Path, destination: &Path) -> io::Result<()> {
    preflight(destination)?;
    if let Some(permissions) = existing_permissions(destination, &fs::metadata(temp)?)? {
        // Re-read current metadata and apply mode after writing, immediately before rename.
        fs::set_permissions(temp, permissions)?;
    }
    fs::rename(temp, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::PathBuf,
    };

    fn directory() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "a2f-unix-mode-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn existing_0600_is_preserved_before_rename() {
        let dir = directory();
        let destination = dir.join("plan.md");
        let temp = dir.join("temp");
        fs::write(&destination, "private old export").unwrap();
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o600)).unwrap();
        let old = fs::metadata(&destination).unwrap();
        fs::write(&temp, "new complete export").unwrap();
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o644)).unwrap();
        commit(&temp, &destination).unwrap();
        let new = fs::metadata(&destination).unwrap();
        assert_eq!(new.mode() & 0o7777, 0o600);
        assert_eq!((new.uid(), new.gid()), (old.uid(), old.gid()));
        assert_eq!(
            fs::read_to_string(&destination).unwrap(),
            "new complete export"
        );
        assert!(!temp.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn staging_uses_existing_mode_before_writing_private_content() {
        let dir = directory();
        let destination = dir.join("plan.md");
        let temp = dir.join("temp");
        fs::write(&destination, "old").unwrap();
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o600)).unwrap();
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .unwrap();
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o644)).unwrap();
        prepare_staged(&file, &destination).unwrap();
        assert_eq!(file.metadata().unwrap().mode() & 0o7777, 0o600);
        file.write_all(b"complete").unwrap();
        file.sync_all().unwrap();
        drop(file);
        commit(&temp, &destination).unwrap();
        assert_eq!(fs::read_to_string(&destination).unwrap(), "complete");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn new_destination_keeps_the_staged_creation_mode() {
        let dir = directory();
        let destination = dir.join("new.md");
        let temp = dir.join("temp");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .unwrap();
        let before = file.metadata().unwrap().mode() & 0o7777;
        prepare_staged(&file, &destination).unwrap();
        assert_eq!(file.metadata().unwrap().mode() & 0o7777, before);
        file.write_all(b"new").unwrap();
        file.sync_all().unwrap();
        drop(file);
        commit(&temp, &destination).unwrap();
        assert_eq!(fs::metadata(&destination).unwrap().mode() & 0o7777, before);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_staged_file_does_not_change_old_content_or_mode() {
        let dir = directory();
        let destination = dir.join("plan.md");
        fs::write(&destination, "private old export").unwrap();
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(commit(&dir.join("missing"), &destination).is_err());
        assert_eq!(
            fs::read_to_string(&destination).unwrap(),
            "private old export"
        );
        assert_eq!(fs::metadata(&destination).unwrap().mode() & 0o7777, 0o600);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn owner_and_group_mismatch_are_refused_without_chown() {
        assert!(require_matching_ownership(42, 7, 42, 7).is_ok());
        for (uid, gid) in [(43, 7), (42, 8)] {
            let error = require_matching_ownership(42, 7, uid, gid).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            assert!(error.to_string().contains("overwrite refused"));
        }
    }

    #[test]
    fn commit_rechecks_the_current_destination_mode() {
        let dir = directory();
        let destination = dir.join("plan.md");
        let temp = dir.join("temp");
        fs::write(&destination, "old").unwrap();
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o600)).unwrap();
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .unwrap();
        prepare_staged(&file, &destination).unwrap();
        file.write_all(b"new").unwrap();
        file.sync_all().unwrap();
        drop(file);
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o640)).unwrap();
        commit(&temp, &destination).unwrap();
        assert_eq!(fs::metadata(&destination).unwrap().mode() & 0o7777, 0o640);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replacing_a_symlink_does_not_change_or_copy_its_targets_metadata() {
        let dir = directory();
        let destination = dir.join("link.md");
        let target = dir.join("target.md");
        let temp = dir.join("temp");
        fs::write(&target, "private target").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&target, &destination).unwrap();
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .unwrap();
        let staged_mode = 0o600;
        prepare_staged(&file, &destination).unwrap();
        assert_eq!(file.metadata().unwrap().mode() & 0o7777, staged_mode);
        file.write_all(b"new link entry").unwrap();
        file.sync_all().unwrap();
        drop(file);
        commit(&temp, &destination).unwrap();
        assert!(
            fs::symlink_metadata(&destination)
                .unwrap()
                .file_type()
                .is_file()
        );
        assert_eq!(
            fs::metadata(&destination).unwrap().mode() & 0o7777,
            staged_mode
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "private target");
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o7777, 0o600);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn readonly_0444_is_refused_before_private_content_is_written() {
        let dir = directory();
        let destination = dir.join("plan.md");
        let temp = dir.join("temp");
        fs::write(&destination, "old private export").unwrap();
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o444)).unwrap();
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .unwrap();
        let error = prepare_staged(&file, &destination).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(file.metadata().unwrap().len(), 0);
        assert_eq!(
            fs::read_to_string(&destination).unwrap(),
            "old private export"
        );
        assert_eq!(fs::metadata(&destination).unwrap().mode() & 0o7777, 0o444);
        drop(file);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn commit_cannot_bypass_write_access_revoked_after_staging() {
        let dir = directory();
        let destination = dir.join("plan.md");
        let temp = dir.join("temp");
        fs::write(&destination, "old private export").unwrap();
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&temp, "new complete export").unwrap();
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o444)).unwrap();
        assert_eq!(
            commit(&temp, &destination).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(
            fs::read_to_string(&destination).unwrap(),
            "old private export"
        );
        assert_eq!(fs::metadata(&destination).unwrap().mode() & 0o7777, 0o444);
        assert!(temp.exists());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn readonly_0444_is_rejected_before_a_stage_is_created() {
        let dir = directory();
        let destination = dir.join("plan.md");
        let temp = dir.join("temp");
        fs::write(&destination, "old").unwrap();
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o444)).unwrap();
        let error = create_staged(&temp, &destination).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(!temp.exists());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn existing_stage_is_created_private_before_metadata_preparation() {
        let dir = directory();
        let destination = dir.join("plan.md");
        let temp = dir.join("temp");
        fs::write(&destination, "old").unwrap();
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o644)).unwrap();
        let file = create_staged(&temp, &destination).unwrap();
        assert_eq!(file.metadata().unwrap().mode() & 0o077, 0);
        assert_eq!(file.metadata().unwrap().len(), 0);
        prepare_staged(&file, &destination).unwrap();
        assert_eq!(file.metadata().unwrap().mode() & 0o7777, 0o644);
        drop(file);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn new_stage_keeps_the_normal_creation_mode() {
        let dir = directory();
        let destination = dir.join("new.md");
        let temp = dir.join("temp");
        let control = dir.join("ordinary");
        fs::write(&control, "").unwrap();
        let file = create_staged(&temp, &destination).unwrap();
        assert_eq!(
            file.metadata().unwrap().mode() & 0o7777,
            fs::metadata(&control).unwrap().mode() & 0o7777
        );
        drop(file);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn readonly_symlink_target_cannot_bypass_write_access() {
        let dir = directory();
        let destination = dir.join("link.md");
        let target = dir.join("target.md");
        let temp = dir.join("temp");
        fs::write(&target, "private target").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o444)).unwrap();
        std::os::unix::fs::symlink(&target, &destination).unwrap();
        assert_eq!(
            create_staged(&temp, &destination).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(!temp.exists());
        assert!(
            fs::symlink_metadata(&destination)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "private target");
        fs::remove_dir_all(dir).unwrap();
    }
}
