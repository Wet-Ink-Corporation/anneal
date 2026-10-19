//! A wrapper's options read as its getopt reads them: short options clustered after one `-`
//! (`-iu root`), a value letter taking the rest of its cluster or else the next word (`-uroot`,
//! `-u root`), an optional letter (getopt's `::`) taking only the rest of its cluster
//! (`xargs -eE`), and long options with their value after `=` or in the next word, written out
//! or abbreviated to a unique prefix. Each wrapper's table is checked against the real binaries
//! by `tests/fixtures/wrappers/probes.txt`.
//!
//! A letter or long option a table does not hold is read as a flag: the versions the tables
//! name refuse it and run nothing, so any reading of what follows is safe for them. A later
//! version that adds a value option is a gap until its table learns it. Where implementations
//! differ (GNU and BSD `env` and `xargs`, `sudo` and `doas`, released and upstream versions) a
//! table holds their union; none of them reads a letter as a flag that another reads as taking
//! a value. A table that holds `-` as a letter (BSD `env`) reads a `--X` word that names none
//! of its long options as a cluster, as BSD getopt does.

use super::{is_option, skip_one};
use crate::hook::guard::shell::Word;

/// What an option takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Arg {
    /// Nothing: a flag.
    No,
    /// A value: the rest of its cluster (or after `=`), else the next word.
    Required,
    /// A value only in its own word: the rest of its cluster, or after `=`.
    Optional,
}

/// Where a wrapper stops reading options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Order {
    /// At the first operand (getopt's leading `+`, or POSIX order): the rest is the command.
    Stop,
    /// Nowhere before `--`: options and operands mix (GNU getopt's default).
    Permute,
}

/// One wrapper's options.
#[derive(Debug)]
pub(super) struct Grammar {
    /// The short options as a getopt option string, without a leading `+`: each letter, then
    /// `:` when it takes a value or `::` when its value is optional.
    pub(super) short: &'static str,
    /// The long options by name, without their `--`.
    pub(super) long: &'static [(&'static str, Arg)],
    /// Where options end.
    pub(super) order: Order,
}

/// An option's name as read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Name {
    /// A short option's letter.
    Short(char),
    /// A long option the table holds, by its full name.
    Long(&'static str),
    /// A long option the table does not hold, or an abbreviation of several.
    Unknown,
}

/// One option read.
#[derive(Debug, Clone, Copy)]
pub(super) struct Parsed<'a> {
    /// Its name.
    pub(super) name: Name,
    /// Its value: the rest of its cluster, after `=`, or the next word's whole text; `None`
    /// when it takes none or the words ran out.
    pub(super) value: Option<&'a str>,
    /// The words after the option and its value.
    pub(super) after: &'a [Word],
}

/// A wrapper's words, read.
#[derive(Debug)]
pub(super) struct Read<'a> {
    /// Its options, in order.
    pub(super) options: Vec<Parsed<'a>>,
    /// Where the options stopped: the first operand on, for [`Order::Stop`] (a `--` that ends
    /// them dropped); the words after `--`, or none, for [`Order::Permute`].
    pub(super) rest: &'a [Word],
}

impl<'a> Read<'a> {
    /// Whether the options hold the short letter `short` or the long option `long`.
    pub(super) fn holds(&self, short: char, long: &str) -> bool {
        self.named(short, long).next().is_some()
    }

    /// Each option named by the short letter `short` or the long option `long`, in order.
    pub(super) fn named<'s>(
        &'s self,
        short: char,
        long: &'s str,
    ) -> impl Iterator<Item = Parsed<'a>> + 's {
        self.options
            .iter()
            .filter(move |option| match option.name {
                Name::Short(letter) => letter == short,
                Name::Long(name) => name == long,
                Name::Unknown => false,
            })
            .copied()
    }
}

impl Grammar {
    /// What the short option `letter` takes; `None` when the table does not hold it.
    fn short_arg(&self, letter: char) -> Option<Arg> {
        let mut chars = self.short.chars().peekable();
        while let Some(current) = chars.next() {
            let mut colons = 0_u8;
            while chars.next_if_eq(&':').is_some() {
                colons = colons.saturating_add(1);
            }
            if current == letter && current != ':' {
                return Some(match colons {
                    0 => Arg::No,
                    1 => Arg::Required,
                    _ => Arg::Optional,
                });
            }
        }
        None
    }

    /// The long option `written` (without `--`) names: its exact name, else the one option it
    /// abbreviates; `None` when it names none or several.
    fn long_option(&self, written: &str) -> Option<(&'static str, Arg)> {
        if let Some(exact) = self.long.iter().find(|(name, _)| *name == written) {
            return Some(*exact);
        }
        let mut matches = self
            .long
            .iter()
            .filter(|(name, _)| name.starts_with(written));
        match (matches.next(), matches.next()) {
            (Some(only), None) => Some(*only),
            _ => None,
        }
    }

    /// Reads the options of `words`.
    pub(super) fn parse<'a>(&self, words: &'a [Word]) -> Read<'a> {
        let mut options = Vec::new();
        let mut rest = words;
        while let [first, tail @ ..] = rest {
            if first.text == "--" {
                return Read {
                    options,
                    rest: tail,
                };
            }
            if !is_option(first) {
                match self.order {
                    Order::Stop => return Read { options, rest },
                    Order::Permute => {
                        rest = tail;
                        continue;
                    }
                }
            }
            rest = match first.text.strip_prefix("--") {
                Some(long) if !self.dash_cluster(long) => self.long_word(long, tail, &mut options),
                _ => self.cluster(first.text.get(1..).unwrap_or_default(), tail, &mut options),
            };
        }
        let rest = match self.order {
            Order::Stop => rest,
            Order::Permute => &[],
        };
        Read { options, rest }
    }

    /// Whether the word `--long` is read as the letter `-` clustered with others: the table holds
    /// `-` as a letter and `long` names none of its long options.
    fn dash_cluster(&self, long: &str) -> bool {
        let written = long.split_once('=').map_or(long, |(name, _)| name);
        self.short_arg('-').is_some() && self.long_option(written).is_none()
    }

    /// Reads the long option `long` (`name` or `name=value`), followed by `tail`; returns the
    /// words after it and its value.
    fn long_word<'a>(
        &self,
        long: &'a str,
        tail: &'a [Word],
        options: &mut Vec<Parsed<'a>>,
    ) -> &'a [Word] {
        let (written, attached) = match long.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (long, None),
        };
        let (name, value, after) = match self.long_option(written) {
            Some((name, Arg::Required)) if attached.is_none() => (
                Name::Long(name),
                tail.first().map(|word| word.text.as_str()),
                skip_one(tail),
            ),
            Some((name, _)) => (Name::Long(name), attached, tail),
            None => (Name::Unknown, attached, tail),
        };
        options.push(Parsed { name, value, after });
        after
    }

    /// Reads the short options clustered in `letters`, followed by `tail`; returns the words
    /// after the cluster and any value it took from them.
    fn cluster<'a>(
        &self,
        letters: &'a str,
        tail: &'a [Word],
        options: &mut Vec<Parsed<'a>>,
    ) -> &'a [Word] {
        for (at, letter) in letters.char_indices() {
            let attached = letters
                .get(at.saturating_add(letter.len_utf8())..)
                .unwrap_or_default();
            let name = Name::Short(letter);
            match self.short_arg(letter).unwrap_or(Arg::No) {
                Arg::No => options.push(Parsed {
                    name,
                    value: None,
                    after: tail,
                }),
                Arg::Optional => {
                    let value = (!attached.is_empty()).then_some(attached);
                    options.push(Parsed {
                        name,
                        value,
                        after: tail,
                    });
                    return tail;
                }
                Arg::Required if !attached.is_empty() => {
                    options.push(Parsed {
                        name,
                        value: Some(attached),
                        after: tail,
                    });
                    return tail;
                }
                Arg::Required => {
                    let after = skip_one(tail);
                    options.push(Parsed {
                        name,
                        value: tail.first().map(|word| word.text.as_str()),
                        after,
                    });
                    return after;
                }
            }
        }
        tail
    }
}

#[cfg(test)]
mod tests {
    use super::{Arg, Grammar, Name, Order, Read};
    use crate::hook::guard::shell::{self, Word};

    const STOP: Grammar = Grammar {
        short: "ab:c::",
        long: &[
            ("bee", Arg::Required),
            ("bell", Arg::No),
            ("cat", Arg::Optional),
        ],
        order: Order::Stop,
    };

    const PERMUTE: Grammar = Grammar {
        order: Order::Permute,
        ..STOP
    };

    fn words(command: &str) -> Vec<Word> {
        let mut commands = shell::parse(command).expect("the command parses");
        commands.remove(0).words
    }

    /// Each option as its name and value, and the texts of the words left.
    fn summary(read: &Read<'_>) -> (Vec<(Name, Option<String>)>, Vec<String>) {
        let options = read
            .options
            .iter()
            .map(|option| (option.name, option.value.map(str::to_owned)))
            .collect();
        let rest = read.rest.iter().map(|word| word.text.clone()).collect();
        (options, rest)
    }

    fn short(letter: char, value: Option<&str>) -> (Name, Option<String>) {
        (Name::Short(letter), value.map(str::to_owned))
    }

    fn texts(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|text| (*text).to_owned()).collect()
    }

    #[test]
    fn a_value_letter_ending_a_cluster_takes_the_next_word() {
        let words = words("-ab x y");

        let read = STOP.parse(&words);

        assert_eq!(
            summary(&read),
            (vec![short('a', None), short('b', Some("x"))], texts(&["y"]))
        );
    }

    #[test]
    fn a_value_letter_inside_a_cluster_takes_the_rest_of_it() {
        let words = words("-abxa y");

        let read = STOP.parse(&words);

        assert_eq!(
            summary(&read),
            (
                vec![short('a', None), short('b', Some("xa"))],
                texts(&["y"])
            )
        );
    }

    #[test]
    fn an_optional_letter_takes_only_the_rest_of_its_cluster() {
        let attached = words("-cab y");
        let alone = words("-c y");

        assert_eq!(
            summary(&STOP.parse(&attached)),
            (vec![short('c', Some("ab"))], texts(&["y"]))
        );
        assert_eq!(
            summary(&STOP.parse(&alone)),
            (vec![short('c', None)], texts(&["y"]))
        );
    }

    #[test]
    fn a_letter_the_table_does_not_hold_is_a_flag() {
        let words = words("-zb x y");

        let read = STOP.parse(&words);

        assert_eq!(
            summary(&read),
            (vec![short('z', None), short('b', Some("x"))], texts(&["y"]))
        );
    }

    #[test]
    fn a_long_option_takes_its_value_after_equals_or_in_the_next_word() {
        let spaced = words("--bee x y");
        let attached = words("--bee=x y");
        let optional = words("--cat y");

        let bee = (Name::Long("bee"), Some("x".to_owned()));
        assert_eq!(
            summary(&STOP.parse(&spaced)),
            (vec![bee.clone()], texts(&["y"]))
        );
        assert_eq!(summary(&STOP.parse(&attached)), (vec![bee], texts(&["y"])));
        assert_eq!(
            summary(&STOP.parse(&optional)),
            (vec![(Name::Long("cat"), None)], texts(&["y"]))
        );
    }

    #[test]
    fn a_unique_abbreviation_names_its_option_and_a_shared_one_none() {
        let unique = words("--bel x");
        let abbreviated = words("--ca y");
        let shared = words("--be x y");

        assert_eq!(
            summary(&STOP.parse(&unique)),
            (vec![(Name::Long("bell"), None)], texts(&["x"]))
        );
        assert_eq!(
            summary(&STOP.parse(&abbreviated)),
            (vec![(Name::Long("cat"), None)], texts(&["y"]))
        );
        assert_eq!(
            summary(&STOP.parse(&shared)),
            (vec![(Name::Unknown, None)], texts(&["x", "y"]))
        );
    }

    #[test]
    fn options_stop_at_the_first_operand_or_after_double_dash() {
        let operand = words("-a y -b x");
        let dashes = words("-a -- -b x");

        assert_eq!(
            summary(&STOP.parse(&operand)),
            (vec![short('a', None)], texts(&["y", "-b", "x"]))
        );
        assert_eq!(
            summary(&STOP.parse(&dashes)),
            (vec![short('a', None)], texts(&["-b", "x"]))
        );
    }

    #[test]
    fn permuted_options_are_read_past_operands_up_to_double_dash() {
        let words = words("y -b x z -a -- -b w");

        let read = PERMUTE.parse(&words);

        assert_eq!(
            summary(&read),
            (
                vec![short('b', Some("x")), short('a', None)],
                texts(&["-b", "w"])
            )
        );
    }
}
