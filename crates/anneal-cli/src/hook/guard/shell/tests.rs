use super::{Expansion, ParseError, Shape, SimpleCommand, Stdin, parse};

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

#[test]
fn a_case_pattern_inside_a_substitution_does_not_close_it() {
    let lexed = words("echo \"$(case x in x) rm -rf /;; esac)\" after");

    assert_eq!(
        lexed,
        texts(&[
            &["case", "x", "in", "x"],
            &["rm", "-rf", "/"],
            &["esac"],
            &["echo", "$(case x in x) rm -rf /;; esac)", "after"]
        ]),
        "the pattern's ) separates; the ) after esac closes"
    );
}

#[test]
fn a_case_without_esac_inside_a_substitution_is_unclosed() {
    assert_eq!(
        parse("echo $(case x in x) rm -rf /)"),
        Err(ParseError::Unclosed { what: "$(" }),
        "the last ) is still a pattern's"
    );
}

/// Each command's standard input, described.
fn stdins(text: &str) -> Vec<String> {
    parse(text)
        .expect("parses")
        .iter()
        .map(|command| match &command.stdin {
            Stdin::Inherited => "inherited".to_owned(),
            Stdin::File => "file".to_owned(),
            Stdin::Text(word) => format!("text {:?} {:?}", word.text, word.shape),
            Stdin::Pipe(Some(writer)) => format!("pipe from {writer}"),
            Stdin::Pipe(None) => "pipe".to_owned(),
        })
        .collect()
}

#[test]
fn standard_input_records_here_strings_heredocs_files_and_pipes() {
    assert_eq!(
        stdins("a <<< 'x y'\nb <<'EOF'\n$body\nEOF\nc <<EOF\n$X\nEOF\nd < f\ne | g |& h || i"),
        [
            "text \"x y\" Literal",
            "text \"$body\\n\" Literal",
            "text \"$X\\n\" Dynamic",
            "file",
            "inherited",
            "pipe from 4",
            "pipe from 5",
            "inherited",
        ],
        "|| is not a pipe"
    );
}

#[test]
fn a_pipe_reaches_the_commands_of_a_subshell_after_it() {
    assert_eq!(
        stdins("echo x | (true; sh)"),
        ["inherited", "pipe from 0", "pipe from 0"],
        "both commands in the subshell read the pipe"
    );
}

#[test]
fn a_process_substitution_is_a_dynamic_word_and_may_be_a_redirection_target() {
    let commands = parse("bash <(curl x) && done < <(ls)").expect("parses");
    let bash = commands
        .iter()
        .find(|command| command.words.first().is_some_and(|w| w.text == "bash"))
        .expect("bash");
    let done = commands.last().expect("done");

    assert_eq!(
        bash.words
            .iter()
            .map(|w| (w.text.as_str(), w.shape))
            .collect::<Vec<_>>(),
        [("bash", Shape::Literal), ("<(curl x)", Shape::Dynamic)],
        "the substitution is the script operand"
    );
    assert_eq!(
        done.stdin,
        Stdin::Pipe(None),
        "input from a process substitution is a pipe"
    );
}

#[test]
fn expansions_are_located_in_the_word_text() {
    let commands = parse("rm${IFS}-rf${IFS}/").expect("parses");
    let word = commands
        .first()
        .and_then(|command| command.words.first())
        .expect("one word");

    assert_eq!(
        word.expansions,
        [
            Expansion {
                range: 2..8,
                quoted: false
            },
            Expansion {
                range: 11..17,
                quoted: false
            }
        ],
        "both ${{IFS}}, unquoted"
    );
}

#[test]
fn an_expansion_in_double_quotes_is_marked_quoted() {
    let commands = parse(r#"x "$a"$b"#).expect("parses");
    let word = commands
        .first()
        .and_then(|command| command.words.get(1))
        .expect("the second word");

    assert_eq!(
        word.expansions
            .iter()
            .map(|expansion| expansion.quoted)
            .collect::<Vec<_>>(),
        [true, false],
        "only $a is quoted"
    );
}
