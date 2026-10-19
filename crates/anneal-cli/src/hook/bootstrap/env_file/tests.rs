use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use super::{MAX_ENV_FILE_BYTES, export_libexec, export_line};

/// A lookup that answers `CLAUDE_ENV_FILE` with `env_file` and `CLAUDE_PLUGIN_ROOT` with `root`.
fn lookup_of<'a>(
    env_file: Option<&'a Path>,
    root: Option<&'a str>,
) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |var: &str| match var {
        "CLAUDE_ENV_FILE" => env_file.map(OsString::from),
        "CLAUDE_PLUGIN_ROOT" => root.map(OsString::from),
        _ => None,
    }
}

/// A temporary directory and the env file path inside it (not created).
fn scratch() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let file = dir.path().join("sessionstart-hook-0.sh");
    (dir, file)
}

#[test]
fn with_both_variables_the_libexec_is_put_first_on_path() {
    let (_dir, file) = scratch();

    let note = export_libexec(lookup_of(Some(&file), Some("/plugins/anneal")));

    assert_eq!(note, None);
    assert_eq!(
        fs::read_to_string(&file).expect("the env file was written"),
        "export PATH='/plugins/anneal/libexec':\"$PATH\"\n"
    );
}

#[test]
fn without_either_variable_nothing_is_written() {
    let (_dir, file) = scratch();

    let without_root = export_libexec(lookup_of(Some(&file), None));
    let empty_root = export_libexec(lookup_of(Some(&file), Some("")));
    let without_file = export_libexec(lookup_of(None, Some("/plugins/anneal")));

    assert_eq!((without_root, empty_root, without_file), (None, None, None));
    assert!(!file.exists(), "no env file is created");
}

#[test]
fn a_second_session_start_does_not_repeat_the_line() {
    let (_dir, file) = scratch();
    fs::write(&file, "export FOO=1\n").expect("an earlier hook's export");

    let first = export_libexec(lookup_of(Some(&file), Some("/plugins/anneal")));
    let second = export_libexec(lookup_of(Some(&file), Some("/plugins/anneal")));

    assert_eq!((first, second), (None, None));
    assert_eq!(
        fs::read_to_string(&file).expect("the env file"),
        "export FOO=1\nexport PATH='/plugins/anneal/libexec':\"$PATH\"\n"
    );
}

#[test]
fn a_single_quote_in_the_directory_is_escaped() {
    let line = export_line("/home/o'neil/libexec").expect("a quotable directory");

    assert_eq!(line, "export PATH='/home/o'\\''neil/libexec':\"$PATH\"\n");
}

#[cfg(unix)]
#[test]
#[cfg_attr(miri, ignore = "runs sh")]
fn sh_sourcing_the_line_puts_the_directory_first_on_path() {
    let (_dir, file) = scratch();
    let root = "/tmp/it's a $HOME `id` \"dir\" \\ $(id)";
    assert_eq!(
        export_libexec(lookup_of(Some(&file), Some(root))),
        None,
        "written"
    );

    let output = std::process::Command::new("/bin/sh")
        .args(["-c", ". \"$1\"; printf %s \"$PATH\"", "sh"])
        .arg(&file)
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("sh runs");

    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8"),
        format!("{root}/libexec:/usr/bin:/bin")
    );
}

#[test]
fn a_directory_with_a_control_character_or_colon_is_refused_with_a_note() {
    let (_dir, file) = scratch();

    let newline = export_libexec(lookup_of(Some(&file), Some("/plugins/x\ny")));
    let colon = export_libexec(lookup_of(Some(&file), Some("C:\\plugins\\anneal")));

    assert_eq!(
        newline.as_deref(),
        Some(
            "anneal did not put its libexec on PATH for Bash, so run it as the plugin's \
             libexec/anneal: the plugin directory holds '\\n', which a PATH line cannot carry"
        )
    );
    assert_eq!(
        colon.as_deref(),
        Some(
            "anneal did not put its libexec on PATH for Bash, so run it as the plugin's \
             libexec/anneal: the plugin directory holds ':', which a PATH line cannot carry"
        )
    );
    assert!(!file.exists(), "nothing is written");
}

#[test]
fn a_file_without_a_final_newline_gets_one_before_the_line() {
    let (_dir, file) = scratch();
    fs::write(&file, "export FOO=1").expect("an export without a newline");

    let note = export_libexec(lookup_of(Some(&file), Some("/p")));

    assert_eq!(note, None);
    assert_eq!(
        fs::read_to_string(&file).expect("the env file"),
        "export FOO=1\nexport PATH='/p/libexec':\"$PATH\"\n"
    );
}

#[test]
fn a_file_over_the_cap_is_appended_to_without_the_check() {
    let (_dir, file) = scratch();
    let line = "export PATH='/p/libexec':\"$PATH\"\n";
    let padding = "#".repeat(usize::try_from(MAX_ENV_FILE_BYTES).expect("fits"));
    fs::write(&file, format!("{line}{padding}\n")).expect("an oversized env file");

    let note = export_libexec(lookup_of(Some(&file), Some("/p")));

    assert_eq!(note, None);
    let text = fs::read_to_string(&file).expect("the env file");
    assert_eq!(text, format!("{line}{padding}\n\n{line}"));
}

#[test]
fn an_env_file_that_cannot_be_opened_is_a_note() {
    let dir = tempfile::tempdir().expect("a temporary directory");

    let note = export_libexec(lookup_of(Some(dir.path()), Some("/p")));

    let note = note.expect("a note");
    assert!(
        note.starts_with(
            "anneal did not put its libexec on PATH for Bash, so run it as the plugin's \
             libexec/anneal: opening CLAUDE_ENV_FILE: "
        ),
        "{note}"
    );
}
