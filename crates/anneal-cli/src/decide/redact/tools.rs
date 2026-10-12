//! Short options that carry a secret for the tools known to take one on the command line
//! (`mysql -pX`, `curl -u user:pass`). Other tools' short options stay readable: `mkdir -p` and
//! `docker run -p 8080:80` are what Clef judges.

/// One family of tools and the options whose value is a secret.
#[derive(Debug)]
struct Family {
    /// Program names, matched against a word's last path component.
    names: &'static [&'static str],
    /// Short option letters whose value is a secret, alone or in a cluster (`-su`).
    letters: &'static [char],
    /// Whether the value may be the next word (`-u x`), not only attached (`-ux`).
    separate: bool,
    /// Long option names (without `--`) whose value is a secret; the credential-named ones
    /// (`--password`) are redacted for every program already.
    long: &'static [&'static str],
}

/// The known tools. `mysql -p` alone prompts and `mysql -p db` names a database, so the MySQL
/// family's value counts only attached; for the others `-p pw` is the password.
const FAMILIES: [Family; 5] = [
    Family {
        names: &[
            "mysql",
            "mysqldump",
            "mysqladmin",
            "mysqlimport",
            "mysqlshow",
            "mysqlcheck",
            "mysqlpump",
            "mariadb",
            "mariadb-dump",
            "mariadb-admin",
        ],
        letters: &['p'],
        separate: false,
        long: &[],
    },
    Family {
        names: &["curl"],
        letters: &['u', 'U'],
        separate: true,
        long: &["user", "proxy-user"],
    },
    Family {
        names: &["sshpass"],
        letters: &['p'],
        separate: true,
        long: &[],
    },
    Family {
        names: &[
            "mongo",
            "mongosh",
            "mongodump",
            "mongorestore",
            "mongoexport",
            "mongoimport",
        ],
        letters: &['p'],
        separate: true,
        long: &[],
    },
    Family {
        names: &["redis-cli"],
        letters: &['a'],
        separate: true,
        long: &[],
    },
];

/// A known tool, found by [`known`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tool(&'static Family);

/// Where an option's secret value is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SecretOption {
    /// In the option's own word, after this many bytes (`-p` of `-pX`, `--user=` of
    /// `--user=x`).
    Attached(usize),
    /// The next word is the value.
    NextWord,
}

/// The tool `word` names (its last path component), if it is a known one.
pub(crate) fn known(word: &str) -> Option<Tool> {
    let program = word.rsplit('/').next().unwrap_or(word);
    FAMILIES
        .iter()
        .find(|family| family.names.contains(&program))
        .map(Tool)
}

impl Tool {
    /// Whether `word` is one of this tool's secret options, and where its value is.
    pub(crate) fn secret_option(self, word: &str) -> Option<SecretOption> {
        let family = self.0;
        if let Some(long) = word.strip_prefix("--") {
            return match long.split_once('=') {
                Some((name, _)) if family.long.contains(&name) => {
                    Some(SecretOption::Attached(name.len().saturating_add(3)))
                }
                None if family.long.contains(&long) => Some(SecretOption::NextWord),
                Some(_) | None => None,
            };
        }
        let cluster = word.strip_prefix('-')?;
        // The letters of a cluster up to the secret one; anything else ends the cluster, so
        // `-hdb.private` is not read as `-p`.
        let at = cluster
            .char_indices()
            .take_while(|&(_, c)| c.is_ascii_alphabetic())
            .find(|&(_, c)| family.letters.contains(&c))
            .map(|(at, _)| at)?;
        // The `-` and the letters through the secret one.
        let kept = at.saturating_add(2);
        if kept < word.len() {
            Some(SecretOption::Attached(kept))
        } else if family.separate {
            Some(SecretOption::NextWord)
        } else {
            None
        }
    }
}
