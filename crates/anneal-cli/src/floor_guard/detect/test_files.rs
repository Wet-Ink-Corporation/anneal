//! Which files are tests, and which lines declare a test.

/// True for a test file: under a `tests` directory, or named `*_test.*`, `*.test.*`,
/// `*.spec.*` or `test_*.py`.
pub(super) fn is_test_path(path: &str) -> bool {
    let mut components = path.rsplit('/');
    let name = components.next().unwrap_or(path);
    components.any(|dir| dir == "tests")
        || name.contains("_test.")
        || name.contains(".test.")
        || name.contains(".spec.")
        || (name.starts_with("test_") && extension(name) == Some("py"))
}

/// The text after the last `.` of `path`, if any.
fn extension(path: &str) -> Option<&str> {
    path.rsplit_once('.').map(|(_, ext)| ext)
}

/// The JavaScript and TypeScript extensions, where `it(` and `test(` declare a test.
const SCRIPT: [&str; 8] = ["js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts"];

/// True when `text`, a line of the file at `path`, declares a test in that file's language:
/// `#[test]` or `#[tokio::test` in Rust, `it(` or `test(` in JavaScript and TypeScript,
/// `def test_` in Python.
pub(super) fn is_test_marker(path: &str, text: &str) -> bool {
    let line = text.trim_start();
    match extension(path) {
        Some("rs") => line.starts_with("#[test]") || line.starts_with("#[tokio::test"),
        Some("py") => line.starts_with("def test_") || line.starts_with("async def test_"),
        Some(ext) if SCRIPT.contains(&ext) => line.starts_with("it(") || line.starts_with("test("),
        _ => false,
    }
}
