use super::{Argv, ArgvFault};

fn argv(words: &[&str]) -> Result<Argv, ArgvFault> {
    Argv::try_from(
        words
            .iter()
            .map(|word| (*word).to_owned())
            .collect::<Vec<_>>(),
    )
}

#[test]
fn a_command_keeps_its_words_as_written() {
    let command = argv(&["anneal", "grill", "--quote", "it's  spaced"]).expect("valid");

    assert_eq!(
        command.words(),
        ["anneal", "grill", "--quote", "it's  spaced"]
    );
}

#[test]
fn an_empty_command_or_a_blank_program_is_refused() {
    assert_eq!(
        (argv(&[]), argv(&[" ", "x"])),
        (Err(ArgvFault::Empty), Err(ArgvFault::BlankProgram))
    );
}

#[test]
fn thirty_two_words_pass_and_thirty_three_are_refused() {
    let words = ["w"; 33];

    assert_eq!(
        (
            argv(&words[..32]).map(|command| command.words().len()),
            argv(&words)
        ),
        (Ok(32), Err(ArgvFault::TooManyWords))
    );
}

#[test]
fn a_word_of_512_bytes_passes_and_one_of_513_is_refused() {
    let longest = "a".repeat(512);
    let over = "a".repeat(513);

    assert_eq!(
        (
            argv(&["echo", &longest]).map(|command| command.words().len()),
            argv(&["echo", "ok", &over])
        ),
        (Ok(2), Err(ArgvFault::LongWord(3)))
    );
}

#[test]
fn a_control_character_in_any_word_is_refused() {
    assert_eq!(
        (
            argv(&["echo", "a\nb"]),
            argv(&["echo\0"]),
            argv(&["echo", "x", "\r"])
        ),
        (
            Err(ArgvFault::ControlCharacter(2)),
            Err(ArgvFault::ControlCharacter(1)),
            Err(ArgvFault::ControlCharacter(3))
        )
    );
}
