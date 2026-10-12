//! The working tree's change as `verify risk` reads it: committed, uncommitted and untracked
//! files with their changed lines, against the merge base.

use super::{Touched, working_change};
use crate::test_repo::{ScratchRepo, Setup};

fn touched(path: &str, lines: Option<u64>) -> Touched {
    Touched {
        path: path.to_owned(),
        old_path: None,
        lines,
    }
}

#[test]
fn committed_uncommitted_and_untracked_files_are_read_with_their_changed_lines() -> Setup<()> {
    let repo = ScratchRepo::new()?;
    repo.git(&["checkout", "-q", "-b", "feature"])?;
    repo.write("db/migrations/1.sql", "create table a;\ncreate table b;\n")?;
    repo.commit("a migration")?;
    repo.write("README.md", "changed\n")?;
    repo.write("src/auth/login.rs", "fn a() {}\nfn b() {}\nfn c() {}\n")?;

    let change = working_change(repo.path(), Some("main"))?;

    assert_eq!(change.base, "main");
    assert_eq!(change.merge_base, repo.git(&["rev-parse", "main"])?);
    assert_eq!(
        change.files,
        vec![
            touched("README.md", Some(2)),
            touched("db/migrations/1.sql", Some(2)),
            touched("src/auth/login.rs", Some(3)),
        ],
        "tracked files in diff order, then untracked ones, each added plus removed lines"
    );
    Ok(())
}

#[test]
fn a_rename_keeps_the_path_it_came_from() -> Setup<()> {
    let repo = ScratchRepo::new()?;
    repo.write(
        "src/payments.rs",
        "fn pay() {}\nfn refund() {}\nfn void() {}\n",
    )?;
    repo.commit("payments")?;
    repo.git(&["mv", "src/payments.rs", "src/money.rs"])?;

    let change = working_change(repo.path(), Some("HEAD"))?;

    assert_eq!(
        change.files,
        vec![Touched {
            path: "src/money.rs".to_owned(),
            old_path: Some("src/payments.rs".to_owned()),
            lines: Some(0),
        }],
        "a pure rename changes no line but keeps both paths"
    );
    Ok(())
}

#[test]
fn an_untracked_file_over_the_read_cap_has_no_line_count() -> Setup<()> {
    let repo = ScratchRepo::new()?;
    repo.write("big.txt", &"x\n".repeat(600 * 1024))?;

    let change = working_change(repo.path(), Some("HEAD"))?;

    assert_eq!(
        change.files,
        vec![touched("big.txt", None)],
        "unmeasured, not dropped"
    );
    Ok(())
}
