//! The certainty tables themselves.

use super::readings::{READINGS, Reading};

/// Every option word of `reading`, from all three lists.
fn options(reading: &Reading) -> impl Iterator<Item = &'static str> {
    [reading.flags, reading.values, reading.body]
        .into_iter()
        .flat_map(str::split_ascii_whitespace)
}

#[test]
fn every_table_entry_is_one_option_word_in_one_list() {
    for (name, reading) in &READINGS {
        let all: Vec<&str> = options(reading).collect();
        for (at, option) in all.iter().enumerate() {
            // `-` and one character, `-` alone (su's login), or `--` and a lower-case name
            // holding no `--` (two names written together).
            let shaped = match option.strip_prefix("--") {
                Some(long) => {
                    long.len() > 1
                        && !long.contains("--")
                        && long.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
                }
                None => *option == "-" || (option.starts_with('-') && option.chars().count() == 2),
            };
            assert!(shaped, "{name}: {option:?} is no option word");
            assert!(
                !all.iter().skip(at + 1).any(|other| other == option),
                "{name}: {option:?} is listed twice"
            );
        }
    }
}

#[test]
fn every_shaped_option_is_a_value_option_and_every_list_flag_a_flag() {
    for (name, reading) in &READINGS {
        for (option, _) in reading.shapes {
            assert!(
                reading
                    .values
                    .split_ascii_whitespace()
                    .any(|value| value == *option),
                "{name}: {option:?} has a shape but takes no value"
            );
        }
        for flag in reading.lists.split_ascii_whitespace() {
            assert!(
                reading
                    .flags
                    .split_ascii_whitespace()
                    .any(|item| item == flag),
                "{name}: {flag:?} makes a CPU list but is no flag"
            );
        }
    }
}

#[test]
fn every_wrapper_is_listed_once() {
    for (at, (name, _)) in READINGS.iter().enumerate() {
        assert!(
            !READINGS.iter().skip(at + 1).any(|(other, _)| other == name),
            "{name} is listed twice"
        );
    }
}
