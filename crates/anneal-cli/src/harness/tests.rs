use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The prefix of every harness variable's name.
const PREFIX: &str = "CLAUDE_";

/// A string literal in code, with the line it starts on.
#[derive(Debug, PartialEq, Eq)]
struct Literal {
    line: usize,
    text: String,
}

/// What one source file holds outside its comments and `#[cfg(test)]` items: its string
/// literals, and the modules it declares under `#[cfg(test)]` (`#[cfg(test)] mod name;`).
#[derive(Debug, Default)]
struct Scan {
    literals: Vec<Literal>,
    test_modules: Vec<String>,
}

/// The `#[cfg(test)]` item being skipped: the code it has so far and its bracket depth.
struct TestItem {
    code: String,
    depth: usize,
}

/// A small Rust lexer: enough to tell string literals (plain, byte, C and raw) from comments
/// (line, nested block) and char literals from lifetimes.
struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
}

impl Lexer {
    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.pos + ahead).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek(0)?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
        }
        Some(c)
    }

    fn line_comment(&mut self) {
        while self.peek(0).is_some_and(|c| c != '\n') {
            self.bump();
        }
    }

    fn block_comment(&mut self) {
        let mut depth = 0_usize;
        while let Some(c) = self.bump() {
            match (c, self.peek(0)) {
                ('/', Some('*')) => {
                    self.bump();
                    depth += 1;
                }
                ('*', Some('/')) => {
                    self.bump();
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    /// The text of a `"..."` literal, the cursor on its opening quote. Escapes are kept as
    /// written, which is enough to find a name in it.
    fn string(&mut self) -> String {
        self.bump();
        let mut text = String::new();
        while let Some(c) = self.bump() {
            match c {
                '"' => break,
                '\\' => {
                    text.push(c);
                    text.extend(self.bump());
                }
                _ => text.push(c),
            }
        }
        text
    }

    /// How many `#` follow an `r` at the cursor when they open a raw string, else `None`.
    fn raw_hashes(&self) -> Option<usize> {
        let hashes = (1..=self.chars.len())
            .take_while(|ahead| self.peek(*ahead) == Some('#'))
            .count();
        (self.peek(hashes + 1) == Some('"')).then_some(hashes)
    }

    /// The text of a raw literal opened by `r`, `hashes` `#` and a quote.
    fn raw_string(&mut self, hashes: usize) -> String {
        for _ in 0..hashes + 2 {
            self.bump();
        }
        let close: String = std::iter::once('"')
            .chain(std::iter::repeat_n('#', hashes))
            .collect();
        let mut text = String::new();
        while let Some(c) = self.bump() {
            text.push(c);
            if text.ends_with(&close) {
                text.truncate(text.len() - close.len());
                break;
            }
        }
        text
    }

    /// Skips a char literal (`'x'`, `'\n'`, `'"'`) at the cursor; a lifetime or label is left
    /// for the caller as code. Returns whether it skipped one.
    fn char_literal(&mut self) -> bool {
        let length = match (self.peek(1), self.peek(2)) {
            (Some('\\'), _) => {
                let close = (3..=self.chars.len())
                    .find(|ahead| matches!(self.peek(*ahead), Some('\'') | None))
                    .unwrap_or(self.chars.len());
                close + 1
            }
            (Some(_), Some('\'')) => 3,
            _ => return false,
        };
        for _ in 0..length {
            self.bump();
        }
        true
    }
}

/// Scans `source` for its string literals and `#[cfg(test)]` module declarations.
fn scan(source: &str) -> Scan {
    let mut lexer = Lexer {
        chars: source.chars().collect(),
        pos: 0,
        line: 1,
    };
    let mut found = Scan::default();
    let mut code = String::new();
    let mut test_item: Option<TestItem> = None;
    while let Some(c) = lexer.peek(0) {
        let line = lexer.line;
        let literal = match (c, lexer.peek(1)) {
            ('/', Some('/')) => {
                lexer.line_comment();
                continue;
            }
            ('/', Some('*')) => {
                lexer.block_comment();
                continue;
            }
            ('"', _) => Some(lexer.string()),
            ('r', _) => lexer.raw_hashes().map(|hashes| lexer.raw_string(hashes)),
            ('\'', _) if lexer.char_literal() => continue,
            _ => None,
        };
        if let Some(text) = literal {
            if test_item.is_none() {
                found.literals.push(Literal { line, text });
            }
            continue;
        }
        lexer.bump();
        code.push(c);
        match test_item.as_mut() {
            None if code.ends_with("#[cfg(test)]") => {
                test_item = Some(TestItem {
                    code: String::new(),
                    depth: 0,
                });
            }
            None => {}
            Some(item) => {
                if item.push(c) {
                    found.test_modules.extend(declared_module(&item.code));
                    test_item = None;
                }
            }
        }
    }
    found
}

impl TestItem {
    /// Adds a code character; returns whether it ends the item (a `;` or the closing `}` at
    /// depth zero).
    fn push(&mut self, c: char) -> bool {
        self.code.push(c);
        match c {
            '(' | '[' | '{' => {
                self.depth += 1;
                false
            }
            ')' | ']' => {
                self.depth -= 1;
                false
            }
            '}' => {
                self.depth -= 1;
                self.depth == 0
            }
            ';' => self.depth == 0,
            _ => false,
        }
    }
}

/// The module `item` declares when it is `[pub(..)] mod name;`.
fn declared_module(item: &str) -> Option<String> {
    let words: Vec<&str> = item.split_whitespace().collect();
    match words.as_slice() {
        [.., "mod", name] => name.strip_suffix(';').map(str::to_owned),
        _ => None,
    }
}

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).expect("a readable source directory") {
        let path = entry.expect("a readable directory entry").path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    files
}

/// The directory holding the files of `file`'s child modules.
fn child_dir(file: &Path) -> PathBuf {
    let parent = file.parent().expect("a file in a directory");
    match file.file_stem().and_then(|stem| stem.to_str()) {
        Some("lib" | "main") => parent.to_path_buf(),
        Some(stem) => parent.join(stem),
        None => panic!("{} has no stem", file.display()),
    }
}

/// Every `CLAUDE_` literal in production code under `src`, outside the harness module, as
/// `path:line: text` relative to `src`.
fn harness_literals_outside_the_adapter(src: &Path) -> Vec<String> {
    let scans: BTreeMap<PathBuf, Scan> = rust_files(src)
        .into_iter()
        .map(|file| {
            let source = fs::read_to_string(&file).expect("a UTF-8 source file");
            let scan = scan(&source);
            (file, scan)
        })
        .collect();
    let mut excluded = vec![src.join("harness.rs"), src.join("harness")];
    for (file, scan) in &scans {
        let dir = child_dir(file);
        for module in &scan.test_modules {
            excluded.push(dir.join(format!("{module}.rs")));
            excluded.push(dir.join(module));
        }
    }
    scans
        .iter()
        .filter(|(file, _)| !excluded.iter().any(|root| file.starts_with(root)))
        .flat_map(|(file, scan)| {
            let shown = file.strip_prefix(src).unwrap_or(file).display().to_string();
            scan.literals
                .iter()
                .filter(|literal| literal.text.contains(PREFIX))
                .map(move |literal| format!("{shown}:{}: {:?}", literal.line, literal.text))
        })
        .collect()
}

/// The variable Claude Code passes `userConfig.<key>` in.
fn option_var(key: &str) -> String {
    let key: String = key
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => c.to_ascii_uppercase(),
            _ => '_',
        })
        .collect();
    format!("CLAUDE_PLUGIN_OPTION_{key}")
}

#[test]
fn the_plugin_option_variables_are_their_user_config_keys_upper_cased() {
    let derived = [
        option_var("cf_account_id"),
        option_var("cf_api_token"),
        option_var(super::MODE_KEY),
    ];

    assert_eq!(
        derived,
        [super::ACCOUNT_VAR, super::TOKEN_VAR, super::MODE_VAR]
    );
}

#[test]
fn no_claude_literal_appears_outside_the_harness_module() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    let found = harness_literals_outside_the_adapter(&src);

    assert!(
        found.is_empty(),
        "harness variable names belong in `crate::harness`; found:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_scan_finds_literals_in_code_and_skips_comments() {
    let source = "// \"CLAUDE_A\"\n/* \"CLAUDE_B\" /* nested */ \"CLAUDE_C\" */\n\
                  let q = '\"'; let l: &'a str = \"CLAUDE_D\";\n\
                  let r = r#\"CLAUDE_E \"quoted\"\"#; let b = b\"CLAUDE_F\";";

    let texts: Vec<(usize, String)> = scan(source)
        .literals
        .into_iter()
        .map(|literal| (literal.line, literal.text))
        .collect();

    assert_eq!(
        texts,
        [
            (3, "CLAUDE_D".to_owned()),
            (4, "CLAUDE_E \"quoted\"".to_owned()),
            (4, "CLAUDE_F".to_owned()),
        ]
    );
}

#[test]
fn the_scan_skips_test_items_and_records_test_modules() {
    let source = "#[cfg(test)]\nconst A: [u8; 1] = [0]; const B: &str = \"CLAUDE_B\";\n\
                  #[cfg(test)]\nmod inline { const C: &str = \"CLAUDE_C\"; }\n\
                  #[cfg(test)]\npub(crate) mod fake;\nconst D: &str = \"CLAUDE_D\";";

    let found = scan(source);

    assert_eq!(
        found.literals,
        [
            Literal {
                line: 2,
                text: "CLAUDE_B".to_owned()
            },
            Literal {
                line: 7,
                text: "CLAUDE_D".to_owned()
            },
        ]
    );
    assert_eq!(found.test_modules, ["fake"]);
}
