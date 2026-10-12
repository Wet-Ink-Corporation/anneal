//! Options whose value is a secret although their name holds no credential word, for the tools
//! known to take one on the command line (`mysql -pX`, `curl -u user:pass`, `docker login -p`).
//! Credential-named options (`--password`, `-storepass`) need no row here: the shared rule in
//! `decide::redact::rules` redacts their values for every program. Other tools' short options
//! stay readable: `mkdir -p` and `docker run -p 8080:80` are what Clef judges.

use super::rules::dequote;

/// Where a secret option's value may be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Value {
    /// Only in the option's own word (`-pX`): `mysql -p` alone prompts, `mysql -p db` names a
    /// database.
    Attached,
    /// In the option's word (`-uX`) or the next word (`-u X`).
    AttachedOrNext,
}

/// One family of tools and the options whose value is a secret.
#[derive(Debug)]
struct Family {
    /// Program names, matched against a word's last path component, quotes removed.
    names: &'static [&'static str],
    /// The subcommand that must come first, when the options mean a secret only there
    /// (`docker login -p`, not `docker run -p`).
    subcommand: Option<&'static str>,
    /// Short option letters whose value is a secret, alone or in a cluster (`-su`).
    letters: &'static [char],
    /// Long option names (without `--`) whose value is a secret.
    long: &'static [&'static str],
    value: Value,
}

const MYSQL: [&str; 10] = [
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
];

const MONGO: [&str; 6] = [
    "mongo",
    "mongosh",
    "mongodump",
    "mongorestore",
    "mongoexport",
    "mongoimport",
];

const LDAP: [&str; 9] = [
    "ldapsearch",
    "ldapmodify",
    "ldapadd",
    "ldapdelete",
    "ldapmodrdn",
    "ldapcompare",
    "ldapwhoami",
    "ldappasswd",
    "ldapexop",
];

/// The known tools.
static FAMILIES: [Family; 10] = [
    Family {
        names: &MYSQL,
        subcommand: None,
        letters: &['p'],
        long: &[],
        value: Value::Attached,
    },
    Family {
        names: &["curl"],
        subcommand: None,
        letters: &['u', 'U', 'b'],
        long: &["user", "proxy-user"],
        value: Value::AttachedOrNext,
    },
    Family {
        names: &["sshpass"],
        subcommand: None,
        letters: &['p'],
        long: &[],
        value: Value::AttachedOrNext,
    },
    Family {
        names: &MONGO,
        subcommand: None,
        letters: &['p'],
        long: &[],
        value: Value::AttachedOrNext,
    },
    Family {
        names: &["redis-cli"],
        subcommand: None,
        letters: &['a'],
        long: &[],
        value: Value::AttachedOrNext,
    },
    Family {
        names: &["sqlcmd", "bcp"],
        subcommand: None,
        letters: &['P'],
        long: &[],
        value: Value::AttachedOrNext,
    },
    Family {
        names: &LDAP,
        subcommand: None,
        letters: &['w'],
        long: &[],
        value: Value::AttachedOrNext,
    },
    Family {
        names: &["smbclient", "rpcclient", "smbcacls"],
        subcommand: None,
        letters: &['U'],
        long: &["user"],
        value: Value::AttachedOrNext,
    },
    Family {
        names: &["docker", "podman", "nerdctl", "helm"],
        subcommand: Some("login"),
        letters: &['p'],
        long: &[],
        value: Value::AttachedOrNext,
    },
    Family {
        names: &["gh"],
        subcommand: Some("secret"),
        letters: &['b'],
        long: &["body"],
        value: Value::AttachedOrNext,
    },
];

/// A known tool named in the command or prose, found by [`known`]; its options count once its
/// subcommand, if it has one, has been seen ([`Tool::note`]).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tool {
    family: &'static Family,
    armed: bool,
}

/// Where an option's secret value is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SecretOption {
    /// In the option's own word, after this many bytes (`-p` of `-pX`, `--user=` of
    /// `--user=x`).
    Attached(usize),
    /// The next word is the value.
    NextWord,
}

/// The tool `word` names (its last path component, with shell quotes and backslashes removed,
/// so `"mysql"` and `\mysql` count), if it is a known one.
pub(crate) fn known(word: &str) -> Option<Tool> {
    let plain = dequote(word);
    let program = plain.rsplit_once('/').map_or(&*plain, |(_, last)| last);
    FAMILIES
        .iter()
        .find(|family| family.names.contains(&program))
        .map(|family| Tool {
            family,
            armed: family.subcommand.is_none(),
        })
}

impl Tool {
    /// Arms the tool when `word` is its subcommand.
    pub(crate) fn note(&mut self, word: &str) {
        if self.family.subcommand == Some(dequote(word).as_ref()) {
            self.armed = true;
        }
    }

    /// Whether `word` (quotes already removed) is one of this tool's secret options, and where
    /// its value is.
    pub(crate) fn secret_option(self, word: &str) -> Option<SecretOption> {
        if !self.armed {
            return None;
        }
        let family = self.family;
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
        match family.value {
            _ if kept < word.len() => Some(SecretOption::Attached(kept)),
            Value::AttachedOrNext => Some(SecretOption::NextWord),
            Value::Attached => None,
        }
    }
}
