use super::porcelain_is_clean;

const CAP: u64 = 4096;

fn devices(path: &[u8]) -> bool {
    path.starts_with(b".")
}

#[test]
fn no_entry_is_clean() {
    assert!(porcelain_is_clean(b"", CAP, devices));
}

#[test]
fn untracked_devices_alone_are_clean() {
    assert!(porcelain_is_clean(
        b"?? .bashrc\0?? .claude/settings.json\0",
        CAP,
        devices
    ));
}

#[test]
fn an_untracked_file_that_is_not_a_device_is_dirty() {
    assert!(!porcelain_is_clean(
        b"?? .bashrc\0?? notes.rs\0",
        CAP,
        devices
    ));
}

#[test]
fn a_tracked_change_is_dirty_even_on_a_device_path() {
    assert!(!porcelain_is_clean(b" M .profile\0", CAP, devices));
    assert!(!porcelain_is_clean(b"R  .a\0.b\0", CAP, devices));
}

#[test]
fn output_past_the_cap_is_dirty() {
    assert!(!porcelain_is_clean(b"?? .bashrc\0?? .zshrc\0", 10, devices));
    assert!(porcelain_is_clean(b"?? .bashrc\0", 11, devices));
}

#[cfg(unix)]
#[test]
fn a_character_device_counts_as_a_device_and_a_link_to_one_does_not() -> std::io::Result<()> {
    use super::is_device;
    let dir = tempfile::tempdir()?;
    std::os::unix::fs::symlink("/dev/null", dir.path().join("null"))?;
    std::fs::write(dir.path().join("plain"), "x")?;

    assert!(is_device(std::path::Path::new("/dev"), b"null"));
    assert!(!is_device(dir.path(), b"null"), "a symlink git can commit");
    assert!(!is_device(dir.path(), b"plain"));
    assert!(
        !is_device(dir.path(), b"missing"),
        "unknown counts as a change"
    );
    Ok(())
}
