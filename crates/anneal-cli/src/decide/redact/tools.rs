//! Options whose value is a secret although their name holds no credential word, for the tools
//! known to take one on the command line (`mysql -pX`, `curl -u user:pass`, `docker login -p`),
//! and the subcommands whose first argument is one (`vault login s.…`).
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
    /// Whether the word right after the subcommand is a secret (`vault login s.…`).
    positional: bool,
}

impl Family {
    /// A family whose secret options count everywhere.
    const fn options(
        names: &'static [&'static str],
        letters: &'static [char],
        long: &'static [&'static str],
        value: Value,
    ) -> Self {
        Self {
            names,
            subcommand: None,
            letters,
            long,
            value,
            positional: false,
        }
    }

    /// A family whose secret options count only after `subcommand`.
    const fn under(
        names: &'static [&'static str],
        subcommand: &'static str,
        letters: &'static [char],
        long: &'static [&'static str],
    ) -> Self {
        Self {
            names,
            subcommand: Some(subcommand),
            letters,
            long,
            value: Value::AttachedOrNext,
            positional: false,
        }
    }
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
static FAMILIES: [Family; 13] = [
    Family::options(&MYSQL, &['p'], &[], Value::Attached),
    Family::options(
        &["curl"],
        &['u', 'U', 'b'],
        &["user", "proxy-user"],
        Value::AttachedOrNext,
    ),
    Family::options(&["sshpass"], &['p'], &[], Value::AttachedOrNext),
    Family::options(&MONGO, &['p'], &[], Value::AttachedOrNext),
    Family::options(&["redis-cli"], &['a'], &[], Value::AttachedOrNext),
    Family::options(&["sqlcmd", "bcp"], &['P'], &[], Value::AttachedOrNext),
    Family::options(&LDAP, &['w'], &[], Value::AttachedOrNext),
    Family::options(
        &["smbclient", "rpcclient", "smbcacls"],
        &['U'],
        &["user"],
        Value::AttachedOrNext,
    ),
    Family::options(&["twine"], &['p'], &[], Value::AttachedOrNext),
    Family::under(
        &["docker", "podman", "nerdctl", "helm"],
        "login",
        &['p'],
        &[],
    ),
    Family::under(&["az"], "login", &['p'], &[]),
    Family::under(&["gh"], "secret", &['b'], &["body"]),
    Family {
        positional: true,
        ..Family::under(&["vault"], "login", &[], &[])
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
    /// Arms the tool when `word` is its subcommand. Returns whether the next word is a secret:
    /// the argument of a subcommand that takes one (`vault login`), whatever it looks like.
    pub(crate) fn note(&mut self, word: &str) -> bool {
        if self.armed || self.family.subcommand != Some(dequote(word).as_ref()) {
            return false;
        }
        self.armed = true;
        self.family.positional
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
