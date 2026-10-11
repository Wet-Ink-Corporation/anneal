use super::{ParseError, Shape, SimpleCommand, parse};

/// The words of each command `text` lexes into, as plain text.
fn words(text: &str) -> Vec<Vec<String>> {
    parse(text)
        .expect("parses")
        .iter()
        .map(|command| command.words.iter().map(|w| w.text.clone()).collect())
        .collect()
}

fn texts(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|row| row.iter().map(|w| (*w).to_owned()).collect())
        .collect()
}

#[test]
fn quotes_and_escapes_are_removed_from_words() {
    assert_eq!(
        words(r#"r'm' "-r"f a\ b 'x y'"z""#),
        texts(&[&["rm", "-rf", "a b", "x yz"]]),
        "quoting joins and unquotes"
    );
}

#[test]
fn control_operators_and_newlines_split_commands() {
    assert_eq!(
        words("a 1 && b || c; d | e & f\ng |& h"),
        texts(&[
            &["a", "1"],
            &["b"],
            &["c"],
            &["d"],
            &["e"],
            &["f"],
            &["g"],
            &["h"]
        ]),
        "every operator ends a command"
    );
}

#[test]
fn substitutions_subshells_and_backticks_are_flattened_into_the_list() {
    let lexed = words("echo $(rm -rf /) `git push -f` (cd x; ls) <(diff a b) $((1 + 2))");

    for inner in [
        vec!["rm", "-rf", "/"],
        vec!["git", "push", "-f"],
        vec!["cd", "x"],
        vec!["ls"],
        vec!["diff", "a", "b"],
    ] {
        let inner: Vec<String> = inner.into_iter().map(str::to_owned).collect();
        assert!(lexed.contains(&inner), "{inner:?} in {lexed:?}");
    }
}

#[test]
fn a_word_with_a_substitution_or_variable_is_dynamic() {
    let commands = parse(r#"x "$(echo a)" $HOME ${A:-b} plain"#).expect("parses");
    let outer = commands.last().expect("the outer command");
    let shapes: Vec<Shape> = outer.words.iter().map(|w| w.shape).collect();

    assert_eq!(
        shapes,
        [
            Shape::Literal,
            Shape::Dynamic,
            Shape::Dynamic,
            Shape::Dynamic,
            Shape::Literal
        ],
        "expansions make a word dynamic"
    );
}

#[test]
fn unquoted_globs_braces_and_tilde_make_a_pattern_but_quoted_ones_do_not() {
    let commands = parse(r#"x * ~/a {a,b} '*' "~""#).expect("parses");
    let shapes: Vec<Shape> = commands
        .first()
        .expect("one command")
        .words
        .iter()
        .map(|w| w.shape)
        .collect();

    assert_eq!(
        shapes,
        [
            Shape::Literal,
            Shape::Pattern,
            Shape::Pattern,
            Shape::Pattern,
            Shape::Literal,
            Shape::Literal
        ],
        "only unquoted pattern characters count"
    );
}

#[test]
fn ansi_c_quoting_is_decoded() {
    assert_eq!(
        words(r"$'\x72\155' $'a\tb'"),
        texts(&[&["rm", "a\tb"]]),
        "hex and octal escapes decode"
    );
}

#[test]
fn a_quoted_heredoc_body_is_data_and_never_parsed() {
    let lexed = words("git commit -F - <<'EOF'\nDon't (do) `this` $(rm -rf /)\nEOF\necho done");

    assert_eq!(
        lexed,
        texts(&[&["git", "commit", "-F", "-"], &["echo", "done"]]),
        "the body is skipped, the command after it is read"
    );
}

#[test]
fn an_unquoted_heredoc_body_runs_its_substitutions() {
    let lexed = words("cat <<EOF\nit's $(rm -rf /)\nEOF");

    assert!(
        lexed.contains(&texts(&[&["rm", "-rf", "/"]])[0]),
        "the substitution in the body is a command: {lexed:?}"
    );
}

#[test]
fn a_heredoc_inside_a_substitution_is_read_as_data() {
    let lexed = words("git commit -m \"$(cat <<'EOF'\nIt's done (finally)\nEOF\n)\"");

    assert_eq!(
        lexed,
        texts(&[
            &["cat"],
            &[
                "git",
                "commit",
                "-m",
                "$(cat <<'EOF'\nIt's done (finally)\nEOF\n)"
            ]
        ]),
        "the apostrophe in the body does not open a quote"
    );
}

#[test]
fn write_redirections_are_collected_and_reads_and_dups_are_not() {
    let commands = parse("a > o1 2>> o2 &> o3 >| o4 < in 2>&1 >&o5 <<< here").expect("parses");
    let command: &SimpleCommand = commands.first().expect("one command");
    let writes: Vec<&str> = command.writes.iter().map(|w| w.text.as_str()).collect();

    assert_eq!(writes, ["o1", "o2", "o3", "o4", "o5"], "write targets only");
    assert_eq!(
        command
            .words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>(),
        ["a"],
        "redirections are not words"
    );
}

#[test]
fn comments_are_skipped_but_a_hash_inside_a_word_is_kept() {
    assert_eq!(
        words("echo a#b # rm -rf /\nls"),
        texts(&[&["echo", "a#b"], &["ls"]]),
        "a comment runs to the end of the line"
    );
}

#[test]
fn a_case_pattern_parenthesis_does_not_fail_the_parse() {
    assert!(
        parse("case $1 in start) run;; esac").is_ok(),
        "an unmatched ) is a separator"
    );
}

#[test]
fn unclosed_quotes_and_substitutions_are_errors() {
    let cases = [
        ("echo 'open", ParseError::Unclosed { what: "'" }),
        ("echo \"open", ParseError::Unclosed { what: "\"" }),
        ("echo $(open", ParseError::Unclosed { what: "$(" }),
        ("echo `open", ParseError::Unclosed { what: "`" }),
        ("echo ${open", ParseError::Unclosed { what: "${" }),
        ("echo >", ParseError::MissingRedirectTarget),
    ];
    for (text, expected) in cases {
        assert_eq!(parse(text), Err(expected), "{text}");
    }
}

#[test]
fn nesting_deeper_than_the_cap_is_an_error() {
    let deep = format!("{}x{}", "$(".repeat(40), ")".repeat(40));

    assert_eq!(parse(&deep), Err(ParseError::TooDeep), "40 levels");
}
