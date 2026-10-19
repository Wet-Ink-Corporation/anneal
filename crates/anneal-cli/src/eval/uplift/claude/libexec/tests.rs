use std::fs;

use super::{LauncherTarget, install};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn each_host_the_launcher_knows_maps_to_the_target_it_tries_first() {
    let hosts = [
        ("linux", "x86_64"),
        ("linux", "aarch64"),
        ("macos", "aarch64"),
        ("macos", "x86_64"),
        ("windows", "x86_64"),
    ];

    let placed: Vec<Option<String>> = hosts
        .iter()
        .map(|(os, arch)| {
            LauncherTarget::of(os, arch).map(|target| {
                crate::path_text::slashed(
                    &target.relative().to_string_lossy(),
                    std::path::MAIN_SEPARATOR,
                )
                .into_owned()
            })
        })
        .collect();

    assert_eq!(
        placed,
        [
            "libexec/x86_64-unknown-linux-musl/anneal",
            "libexec/aarch64-unknown-linux-musl/anneal",
            "libexec/aarch64-apple-darwin/anneal",
            "libexec/x86_64-apple-darwin/anneal",
            "libexec/x86_64-pc-windows-msvc/anneal.exe",
        ]
        .map(|path| Some(path.to_owned()))
    );
}

#[test]
fn a_host_the_launcher_does_not_know_has_no_target() {
    let unknown = [
        ("freebsd", "x86_64"),
        ("linux", "riscv64"),
        ("windows", "aarch64"),
        ("macos", "powerpc"),
    ];

    assert_eq!(
        unknown.map(|(os, arch)| LauncherTarget::of(os, arch)),
        [None; 4]
    );
}

#[test]
fn install_copies_the_binary_where_the_launcher_looks() -> TestResult {
    let source = tempfile::tempdir()?;
    let binary = source.path().join("anneal");
    fs::write(&binary, "binary")?;
    let copy = tempfile::tempdir()?;
    let target = LauncherTarget::of("linux", "x86_64").ok_or("a target")?;

    install(&binary, copy.path(), target)?;

    assert_eq!(
        fs::read_to_string(copy.path().join("libexec/x86_64-unknown-linux-musl/anneal"))?,
        "binary"
    );
    Ok(())
}

#[test]
#[cfg(unix)]
fn the_installed_binary_is_executable_and_replaces_a_read_only_one() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let source = tempfile::tempdir()?;
    let binary = source.path().join("anneal");
    fs::write(&binary, "new")?;
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o600))?;
    let copy = tempfile::tempdir()?;
    let placed = copy.path().join("libexec/aarch64-apple-darwin/anneal");
    fs::create_dir_all(placed.parent().ok_or("a parent")?)?;
    fs::write(&placed, "old")?;
    fs::set_permissions(&placed, fs::Permissions::from_mode(0o444))?;
    let target = LauncherTarget::of("macos", "aarch64").ok_or("a target")?;

    install(&binary, copy.path(), target)?;

    assert_eq!(
        (
            fs::read_to_string(&placed)?,
            fs::metadata(&placed)?.permissions().mode() & 0o777
        ),
        ("new".to_owned(), 0o755)
    );
    Ok(())
}
