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
    let sddl: Vec<u16> = "D:P(A;;FA;;;OW)".encode_utf16().chain(Some(0)).collect();
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
fn windows_overwrite_preserves_protected_dacl_and_staging_is_a_negative_control() {
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
        assert_ne!(dacl(&staged), before, "staged inherited DACL must differ");
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
