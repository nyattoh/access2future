use super::*;
use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
        DACL_SECURITY_INFORMATION, GetFileSecurityW, GetSecurityDescriptorControl,
        GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION, SE_DACL_PROTECTED,
        SetFileSecurityW,
    },
};

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

pub(super) fn protect(path: &Path) {
    set_dacl(path, "D:P(A;;FA;;;OW)");
}

pub(super) fn set_dacl(path: &Path, sddl: &str) {
    let sddl: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
    let mut descriptor = std::ptr::null_mut();
    unsafe {
        assert_ne!(
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut()
            ),
            0
        );
        let result = SetFileSecurityW(
            wide(path).as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        );
        let error = SaveError::last_os_error();
        LocalFree(descriptor);
        assert_ne!(result, 0, "{error}");
    }
}

pub(super) fn dacl(path: &Path) -> (Vec<u8>, bool) {
    let mut needed = 0;
    unsafe {
        GetFileSecurityW(
            wide(path).as_ptr(),
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            0,
            &mut needed,
        );
    }
    assert!(needed > 0);
    let mut buffer = vec![0u32; (needed as usize).div_ceil(4)];
    let descriptor = buffer.as_mut_ptr().cast();
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl = std::ptr::null_mut();
    let mut control = 0;
    let mut revision = 0;
    unsafe {
        assert_ne!(
            GetFileSecurityW(
                wide(path).as_ptr(),
                DACL_SECURITY_INFORMATION,
                descriptor,
                needed,
                &mut needed
            ),
            0
        );
        assert_ne!(
            GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted),
            0
        );
        assert_ne!(present, 0);
        assert!(!acl.is_null());
        assert_ne!(
            GetSecurityDescriptorControl(descriptor, &mut control, &mut revision),
            0
        );
        (
            std::slice::from_raw_parts(acl.cast(), (*acl).AclSize as usize).to_vec(),
            control & SE_DACL_PROTECTED != 0,
        )
    }
}

#[test]
fn windows_overwrite_preserves_protected_dacl_before_and_after_staging() {
    let dir = tests::temp_dir("dacl");
    let path = dir.join("plan.md");
    fs::write(&path, "old private export").unwrap();
    protect(&path);
    let before = dacl(&path);
    assert!(before.1);
    write_export_with(&path, "new export", |file, bytes| {
        let staged = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p != &path)
            .unwrap();
        assert_eq!(dacl(&staged), before, "staged DACL must already match");
        file.write_all(bytes)
    })
    .unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "new export");
    assert!(
        dacl(&path) == before,
        "replacement must preserve DACL bytes and protection"
    );
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn windows_new_destination_racer_is_not_overwritten() {
    let dir = tests::temp_dir("racer");
    let path = dir.join("plan.json");
    let result = write_export_with(&path, "our export", |file, bytes| {
        fs::write(&path, "racer")?;
        protect(&path);
        file.write_all(bytes)
    });
    assert!(result.is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "racer");
    assert!(dacl(&path).1);
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn windows_sharing_lock_preserves_bytes_and_dacl_without_debris() {
    let dir = tests::temp_dir("locked");
    let path = dir.join("plan.md");
    fs::write(&path, "previous").unwrap();
    protect(&path);
    let before = dacl(&path);
    let lock = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    assert!(write_export_file(&path, "replacement").is_err());
    drop(lock);
    assert_eq!(fs::read_to_string(&path).unwrap(), "previous");
    assert_eq!(dacl(&path), before);
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn windows_disappeared_destination_can_be_created_without_replacement() {
    let dir = tests::temp_dir("vanished");
    let path = dir.join("plan.md");
    fs::write(&path, "old").unwrap();
    write_export_with(&path, "new", |file, bytes| {
        fs::remove_file(&path)?;
        file.write_all(bytes)
    })
    .unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn windows_restricted_staging_acl_allows_full_overwrite() {
    let dir = tests::temp_dir("no-write-dac");
    let path = dir.join("plan.md");
    for content in ["first export", "complete replacement"] {
        write_export_with(&path, content, |file, bytes| {
            let staged = fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| p != &path)
                .unwrap();
            // Owner Rights explicitly grants read/write/delete but not WRITE_DAC.
            if !path.exists() {
                set_dacl(&staged, "D:P(A;;GRGWSD;;;OW)");
            } else {
                assert_eq!(dacl(&staged), dacl(&path));
            }
            file.write_all(bytes)
        })
        .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), content);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn windows_stage_has_destination_acl_before_first_content_byte() {
    let dir = tests::temp_dir("prewrite-acl");
    let path = dir.join("plan.md");
    fs::write(&path, "old private export").unwrap();
    protect(&path);
    let before = dacl(&path);
    let inherited = dir.join("negative-control");
    fs::write(&inherited, "").unwrap();
    assert_ne!(dacl(&inherited), before);
    fs::remove_file(inherited).unwrap();
    let result = write_export_with(&path, "private replacement", |file, bytes| {
        let staged = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p != &path)
            .unwrap();
        assert_eq!(file.metadata()?.len(), 0);
        assert_eq!(dacl(&staged), before, "ACL must precede content");
        file.write_all(bytes)
    });
    result.unwrap();
    assert_eq!(dacl(&path), before);
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn windows_read_only_dacl_refuses_save_before_writer_and_leaves_no_stage() {
    let dir = tests::temp_dir("read-only-acl");
    let path = dir.join("plan.md");
    fs::write(&path, "old private export").unwrap();
    set_dacl(&path, "D:P(A;;GRSD;;;OW)");
    let before = dacl(&path);
    let mut called = false;
    let result = write_export_with(&path, "replacement", |file, bytes| {
        called = true;
        file.write_all(bytes)
    });
    assert!(
        !called,
        "a denied target must be rejected before staging content"
    );
    assert_eq!(
        result.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "old private export");
    assert_eq!(dacl(&path), before);
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn windows_permission_revoked_during_staging_cannot_use_replacement_fallback() {
    let dir = tests::temp_dir("revoked-acl");
    let path = dir.join("plan.md");
    fs::write(&path, "old private export").unwrap();
    let result = write_export_with(&path, "replacement", |file, bytes| {
        set_dacl(&path, "D:P(A;;GRSD;;;OW)");
        file.write_all(bytes)
    });
    assert_eq!(
        result.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "old private export");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn windows_readonly_attribute_refuses_save_without_staging_content() {
    let dir = tests::temp_dir("readonly-attribute");
    let path = dir.join("plan.md");
    fs::write(&path, "old").unwrap();
    let original = fs::metadata(&path).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&path, readonly).unwrap();
    let mut called = false;
    let result = write_export_with(&path, "new", |file, bytes| {
        called = true;
        file.write_all(bytes)
    });
    assert!(!called);
    assert_eq!(
        result.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "old");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    fs::set_permissions(&path, original).unwrap();
    fs::remove_dir_all(dir).unwrap();
}
