use super::{Setup, script, setup_lines};
use crate::eval::uplift::case::Argv;

/// The setup commands `commands`, each word as written.
fn commands(commands: &[&[&str]]) -> Vec<Argv> {
    commands
        .iter()
        .map(|words| {
            Argv::try_from(
                words
                    .iter()
                    .map(|word| (*word).to_owned())
                    .collect::<Vec<_>>(),
            )
            .expect("a valid command")
        })
        .collect()
}

#[test]
fn setup_puts_the_plugin_libexec_first_on_path_and_quotes_each_word() {
    let commands = commands(&[
        &["anneal", "grill", "confirm", "--quote", "Yes, that's  it."],
        &["anneal", "plan", "approve", "docs/plans/p.md"],
    ]);

    let text = setup_lines(&Setup {
        libexec: "/ws/it's/with/libexec",
        commands: &commands,
    });

    assert_eq!(
        text,
        "PATH='/ws/it'\\''s/with/libexec':\"$PATH\"\n\
         export PATH\n\
         'anneal' 'grill' 'confirm' '--quote' 'Yes, that'\\''s  it.'\n\
         'anneal' 'plan' 'approve' 'docs/plans/p.md'\n"
    );
}

#[test]
fn the_script_copies_the_fixture_dereferenced_and_commits_it() {
    assert_eq!(
        script("/repo/evals/uplift/fixtures/crate"),
        "#!/bin/sh\n\
         # Written by anneal eval uplift: seeds the run's empty workspace with the case's fixture.\n\
         set -eu\n\
         cp -RL '/repo/evals/uplift/fixtures/crate/.' .\n\
         git init -q -b main\n\
         git add -A\n\
         git -c user.name=eval -c user.email=eval@invalid -c commit.gpgsign=false commit -qm \
         fixture\n"
    );
}

#[test]
fn a_single_quote_in_the_fixture_path_is_escaped() {
    let text = script("/home/o'brien/it's/fixtures/x");

    assert!(
        text.contains("\ncp -RL '/home/o'\\''brien/it'\\''s/fixtures/x/.' .\n"),
        "{text}"
    );
}

#[test]
fn the_script_is_lf_only() {
    let text = script("/a\r\nb");

    assert_eq!(
        text.matches('\r').count(),
        1,
        "only the path's own CR: {text:?}"
    );
}

/// The tests that run the script or read its mode, which need a unix shell and file modes.
#[cfg(unix)]
mod on_unix {
    use std::fs;
    use std::path::Path;
    use std::process::{Command, Output};

    use super::super::{SCRIPT_FILE, Setup, write};
    use super::commands;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// `program` with `args` in `dir`, with git's global and system settings out of the way.
    fn run(program: &str, args: &[&str], dir: &Path, home: &Path) -> std::io::Result<Output> {
        Command::new(program)
            .args(args)
            .current_dir(dir)
            .env("HOME", home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
            .output()
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs sh and git")]
    fn sh_runs_the_script_into_a_committed_repository_holding_the_fixture() -> TestResult {
        if !crate::eval::uplift::test_git::available()? {
            return Ok(());
        }
        let repo = tempfile::tempdir()?;
        fs::create_dir_all(repo.path().join("docs"))?;
        fs::write(repo.path().join("docs/guide.md"), "guide\n")?;
        let fixture = repo.path().join("evals/uplift/fixtures/it's");
        fs::create_dir_all(fixture.join("src"))?;
        fs::write(fixture.join("src/lib.rs"), "pub fn f() {}\n")?;
        fs::write(fixture.join(".hidden"), "dot\n")?;
        std::os::unix::fs::symlink("../../../../docs", fixture.join("docs"))?;
        let case_dir = tempfile::tempdir()?;
        write(case_dir.path(), fixture.to_str().ok_or("UTF-8")?, None)?;
        let workspace = tempfile::tempdir()?;
        let home = tempfile::tempdir()?;

        let ran = run(
            "sh",
            &[case_dir.path().join(SCRIPT_FILE).to_str().ok_or("UTF-8")?],
            workspace.path(),
            home.path(),
        )?;

        assert!(ran.status.success(), "{ran:?}");
        let log = run(
            "git",
            &["log", "--format=%s %an <%ae>"],
            workspace.path(),
            home.path(),
        )?;
        let files = run("git", &["ls-files"], workspace.path(), home.path())?;
        assert_eq!(
            (
                String::from_utf8(log.stdout)?,
                String::from_utf8(files.stdout)?,
                fs::symlink_metadata(workspace.path().join("docs"))?.is_dir(),
                fs::read_to_string(workspace.path().join("docs/guide.md"))?,
            ),
            (
                "fixture eval <eval@invalid>\n".to_owned(),
                ".hidden\ndocs/guide.md\nsrc/lib.rs\n".to_owned(),
                true,
                "guide\n".to_owned(),
            )
        );
        Ok(())
    }

    /// `floor-guard` diffs against `origin/main` or `main`, so the fixture's branch is `main`
    /// whatever `init.defaultBranch` says (Tier 3 r6).
    #[test]
    #[cfg_attr(miri, ignore = "runs sh and git")]
    fn the_scaffolded_repository_is_on_branch_main() -> TestResult {
        if !crate::eval::uplift::test_git::available()? {
            return Ok(());
        }
        let fixture = tempfile::tempdir()?;
        fs::write(fixture.path().join("a.txt"), "a\n")?;
        let case_dir = tempfile::tempdir()?;
        write(
            case_dir.path(),
            fixture.path().to_str().ok_or("UTF-8")?,
            None,
        )?;
        let workspace = tempfile::tempdir()?;
        let home = tempfile::tempdir()?;
        fs::write(
            home.path().join("gitconfig"),
            "[init]\n\tdefaultBranch = trunk\n",
        )?;

        let ran = run(
            "sh",
            &[case_dir.path().join(SCRIPT_FILE).to_str().ok_or("UTF-8")?],
            workspace.path(),
            home.path(),
        )?;

        assert!(ran.status.success(), "{ran:?}");
        let branch = run(
            "git",
            &["rev-parse", "--abbrev-ref", "HEAD"],
            workspace.path(),
            home.path(),
        )?;
        assert_eq!(String::from_utf8(branch.stdout)?, "main\n");
        Ok(())
    }

    #[test]
    fn the_written_script_is_executable() -> TestResult {
        use std::os::unix::fs::PermissionsExt;

        let case_dir = tempfile::tempdir()?;

        write(case_dir.path(), "/f", None)?;

        let mode = fs::metadata(case_dir.path().join(SCRIPT_FILE))?
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o755, "{mode:o}");
        Ok(())
    }

    /// A fixture of one file, a `libexec/` holding a fake `anneal` that appends each argument
    /// on its own line to `<home>/anneal.log`, and the script for both with `setup` written.
    struct Seeded {
        case_dir: tempfile::TempDir,
        workspace: tempfile::TempDir,
        home: tempfile::TempDir,
        _fixture: tempfile::TempDir,
        _libexec: tempfile::TempDir,
    }

    impl Seeded {
        fn with(setup: &[&[&str]]) -> Result<Self, Box<dyn std::error::Error>> {
            let fixture = tempfile::tempdir()?;
            fs::write(fixture.path().join("a.txt"), "a\n")?;
            let home = tempfile::tempdir()?;
            let libexec = tempfile::tempdir()?;
            let fake = libexec.path().join("anneal");
            let log = home.path().join("anneal.log");
            fs::write(
                &fake,
                format!(
                    "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\n",
                    log.to_str().ok_or("UTF-8")?
                ),
            )?;
            super::super::make_executable(&fake)?;
            let commands = commands(setup);
            let case_dir = tempfile::tempdir()?;
            write(
                case_dir.path(),
                fixture.path().to_str().ok_or("UTF-8")?,
                Some(&Setup {
                    libexec: libexec.path().to_str().ok_or("UTF-8")?,
                    commands: &commands,
                }),
            )?;
            Ok(Self {
                case_dir,
                workspace: tempfile::tempdir()?,
                home,
                _fixture: fixture,
                _libexec: libexec,
            })
        }

        fn run_script(&self) -> std::io::Result<Output> {
            let script = self.case_dir.path().join(SCRIPT_FILE);
            run(
                "sh",
                &[&script.to_string_lossy()],
                self.workspace.path(),
                self.home.path(),
            )
        }

        fn log(&self) -> std::io::Result<String> {
            fs::read_to_string(self.home.path().join("anneal.log"))
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs sh and git")]
    fn setup_runs_after_the_commit_in_order_with_anneal_from_the_plugin_libexec() -> TestResult {
        if !crate::eval::uplift::test_git::available()? {
            return Ok(());
        }
        let seeded = Seeded::with(&[
            &["git", "tag", "seeded"],
            &["anneal", "grill", "--quote", "Yes, that's  it."],
            &["anneal", "plan"],
        ])?;

        let ran = seeded.run_script()?;

        assert!(ran.status.success(), "{ran:?}");
        let tags = run(
            "git",
            &["tag", "--points-at", "HEAD"],
            seeded.workspace.path(),
            seeded.home.path(),
        )?;
        assert_eq!(
            (String::from_utf8(tags.stdout)?, seeded.log()?),
            (
                "seeded\n".to_owned(),
                "grill\n--quote\nYes, that's  it.\nplan\n".to_owned()
            )
        );
        Ok(())
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs sh and git")]
    fn a_failing_setup_command_fails_the_script_and_stops_the_rest() -> TestResult {
        if !crate::eval::uplift::test_git::available()? {
            return Ok(());
        }
        let seeded = Seeded::with(&[&["false"], &["anneal", "never"]])?;

        let ran = seeded.run_script()?;

        assert_eq!(
            (
                ran.status.success(),
                seeded.log().map_err(|error| error.kind())
            ),
            (false, Err(std::io::ErrorKind::NotFound))
        );
        Ok(())
    }
}
