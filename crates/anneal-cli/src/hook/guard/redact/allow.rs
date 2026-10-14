//! The allowlists: the only argument text the skeleton shows. Each is fixed vocabulary, so no
//! word the command's author chose can reach Clef through one.

/// Most characters of a program word, option name or assignment name shown.
pub(super) const MAX_NAME_CHARS: usize = 64;

/// Arguments kept verbatim wherever they stand: what a destructive command most often names.
const TARGETS: [&str; 21] = [
    "/",
    "/*",
    "~",
    "~/",
    "~/*",
    ".",
    "..",
    "*",
    "./*",
    "{}",
    "HEAD",
    "main",
    "master",
    "origin",
    "origin/main",
    "origin/master",
    "/dev/null",
    "/dev/zero",
    "/dev/random",
    "/dev/urandom",
    "/dev/stdin",
];

/// Per program, the arguments that say what it does to what: its subcommands (and for
/// `kubectl` its resource kinds) that matter to "destructive or irreversible".
const SUBCOMMANDS: [(&str, &[&str]); 17] = [
    ("git", GIT),
    ("gh", GH),
    (
        "cargo",
        &["publish", "yank", "clean", "install", "uninstall", "owner"],
    ),
    ("npm", NPM),
    ("pnpm", NPM),
    ("yarn", NPM),
    ("docker", DOCKER),
    ("podman", DOCKER),
    ("kubectl", KUBECTL),
    ("terraform", TERRAFORM),
    ("tofu", TERRAFORM),
    (
        "helm",
        &["uninstall", "delete", "rollback", "upgrade", "install"],
    ),
    ("aws", AWS),
    (
        "gcloud",
        &[
            "projects",
            "compute",
            "instances",
            "sql",
            "container",
            "clusters",
            "storage",
            "functions",
            "delete",
            "rm",
        ],
    ),
    (
        "az",
        &[
            "group", "vm", "storage", "account", "aks", "sql", "webapp", "delete",
        ],
    ),
    (
        "systemctl",
        &[
            "stop", "disable", "mask", "kill", "restart", "reboot", "poweroff", "halt",
        ],
    ),
    // `find`'s action ends.
    ("find", &[";", "+"]),
];

const GIT: &[&str] = &[
    "push",
    "reset",
    "clean",
    "checkout",
    "switch",
    "restore",
    "branch",
    "rebase",
    "filter-branch",
    "filter-repo",
    "stash",
    "rm",
    "drop",
    "clear",
    "tag",
    "update-ref",
    "reflog",
    "expire",
    "gc",
    "prune",
    "worktree",
    "remove",
    "commit",
    "merge",
    // Rev-list options after `--` (`filter-branch -- --all`), where every word is an operand.
    "--all",
];

const GH: &[&str] = &[
    "pr", "merge", "close", "repo", "delete", "archive", "rename", "release", "issue", "secret",
    "variable", "run", "cancel", "workflow", "disable",
];

const NPM: &[&str] = &[
    "publish",
    "unpublish",
    "deprecate",
    "uninstall",
    "remove",
    "rm",
    "prune",
    "cache",
    "clean",
    "dist-tag",
    "owner",
    "access",
];

const DOCKER: &[&str] = &[
    "rm",
    "rmi",
    "kill",
    "stop",
    "system",
    "volume",
    "network",
    "container",
    "image",
    "builder",
    "prune",
    "push",
    "compose",
    "down",
];

const KUBECTL: &[&str] = &[
    "delete",
    "apply",
    "replace",
    "patch",
    "drain",
    "cordon",
    "scale",
    "rollout",
    "undo",
    "restart",
    "edit",
    "all",
    "ns",
    "namespace",
    "namespaces",
    "pod",
    "pods",
    "deploy",
    "deployment",
    "deployments",
    "statefulset",
    "sts",
    "service",
    "svc",
    "pvc",
    "pv",
    "node",
    "nodes",
    "secret",
    "secrets",
    "configmap",
    "cm",
    "crd",
    "job",
    "cronjob",
];

const TERRAFORM: &[&str] = &[
    "destroy",
    "apply",
    "import",
    "state",
    "rm",
    "mv",
    "taint",
    "untaint",
    "workspace",
    "delete",
    "force-unlock",
];

const AWS: &[&str] = &[
    "s3",
    "rm",
    "rb",
    "ec2",
    "rds",
    "iam",
    "cloudformation",
    "lambda",
    "dynamodb",
    "terminate-instances",
    "delete-stack",
    "delete-db-instance",
    "delete-bucket",
    "delete-function",
    "delete-table",
    "delete-user",
];

/// Per program, the short flags that never take a value: a cluster of only these is kept
/// (`rm -rf`, `git clean -fdx`), any other cluster may hide one (`mysql -psecret`).
const NO_VALUE_FLAGS: [(&str, &str); 12] = [
    ("rm", "rRfidvI"),
    ("git", "fdxXnqvDu"),
    ("docker", "afqv"),
    ("podman", "afqv"),
    ("kubectl", "A"),
    ("cp", "rRfaivn"),
    ("mv", "fivn"),
    ("chmod", "Rfv"),
    ("chown", "Rfv"),
    ("chgrp", "Rfv"),
    ("ln", "sfvn"),
    ("shred", "fuvz"),
];

/// Per program, single-dash long flags kept as written: flag names, never values.
const EXACT_FLAGS: [(&str, &[&str]); 3] = [
    ("terraform", TERRAFORM_FLAGS),
    ("tofu", TERRAFORM_FLAGS),
    (
        "find",
        &[
            "-delete",
            "-exec",
            "-execdir",
            "-ok",
            "-okdir",
            "-name",
            "-iname",
            "-path",
            "-type",
            "-maxdepth",
            "-mindepth",
            "-print",
            "-print0",
        ],
    ),
];

const TERRAFORM_FLAGS: &[&str] = &["-auto-approve", "-destroy", "-force"];

/// Per program, the `key=value` operands whose key is kept, and whose value is kept only when
/// it is a [`target`].
const KEYED: [(&str, &[&str]); 1] = [("dd", &["if", "of"])];

/// The table row of `program`, if any.
fn row<T: Copy>(table: &[(&str, T)], program: Option<&str>) -> Option<T> {
    let program = program?;
    table
        .iter()
        .find(|(name, _)| *name == program)
        .map(|&(_, value)| value)
}

/// Whether `text` is a program's name shown as written: 1 to 64 of `[A-Za-z0-9._+-]` (a name,
/// so no path).
pub(super) fn is_program(text: &str) -> bool {
    (1..=MAX_NAME_CHARS).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
}

/// Whether any table names `text` as a word: a target, a subcommand, an exact flag or a key.
#[cfg(test)]
pub(super) fn names(text: &str) -> bool {
    TARGETS.contains(&text)
        || SUBCOMMANDS.iter().any(|(_, words)| words.contains(&text))
        || EXACT_FLAGS.iter().any(|(_, flags)| flags.contains(&text))
        || KEYED.iter().any(|(_, keys)| keys.contains(&text))
}

/// Whether `name` is a long option's name shown as written: 1 to 64 of `[A-Za-z0-9._-]`.
pub(super) fn is_option_name(name: &str) -> bool {
    (1..=MAX_NAME_CHARS).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// Whether `text` is a sensitive target, kept wherever it stands: one of [`TARGETS`] or a
/// block device ([`is_device`]).
pub(super) fn is_target(text: &str) -> bool {
    TARGETS.contains(&text) || is_device(text)
}

/// Whether `text` is a subcommand `program` keeps.
pub(super) fn is_subcommand(program: Option<&str>, text: &str) -> bool {
    row(&SUBCOMMANDS, program).is_some_and(|words| words.contains(&text))
}

/// Whether the short option cluster `letters` (after its `-`) is kept for `program`: one ASCII
/// letter or digit, or only flags that take no value.
pub(super) fn is_flag_cluster(program: Option<&str>, letters: &str) -> bool {
    let mut chars = letters.chars();
    match (chars.next(), chars.next()) {
        (Some(only), None) => only.is_ascii_alphanumeric(),
        (Some(_), Some(_)) => row(&NO_VALUE_FLAGS, program)
            .is_some_and(|flags| letters.chars().all(|c| flags.contains(c))),
        (None, _) => false,
    }
}

/// Whether the single-dash word `text` is one of `program`'s exact flags.
pub(super) fn is_exact_flag(program: Option<&str>, text: &str) -> bool {
    row(&EXACT_FLAGS, program).is_some_and(|flags| flags.contains(&text))
}

/// Whether `key` (of a `key=value` operand) is kept for `program`.
pub(super) fn is_key(program: Option<&str>, key: &str) -> bool {
    row(&KEYED, program).is_some_and(|keys| keys.contains(&key))
}

/// A block device: `/dev/` and `sd`, `hd`, `vd` or `xvd` with 1 or 2 letters and up to 3
/// digits, `nvme<n>n<n>[p<n>]`, `mmcblk<n>[p<n>]`, `disk<n>[s<n>]`, `rdisk<n>[s<n>]` or
/// `md<n>`, each number 1 to 3 digits.
fn is_device(text: &str) -> bool {
    let Some(name) = text.strip_prefix("/dev/") else {
        return false;
    };
    let lettered = ["xvd", "sd", "hd", "vd"]
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix))
        .and_then(|rest| run(rest, |c| c.is_ascii_lowercase(), 1, 2))
        .and_then(|rest| run(rest, |c| c.is_ascii_digit(), 0, 3));
    let numbered = |prefix: &str, inner: Option<char>, part: Option<char>| {
        let rest = number(name.strip_prefix(prefix)?)?;
        let rest = match inner {
            Some(c) => number(rest.strip_prefix(c)?)?,
            None => rest,
        };
        match (part, rest.strip_prefix(|c| Some(c) == part)) {
            (Some(_), Some(after)) => number(after),
            _ => Some(rest),
        }
    };
    [
        lettered,
        numbered("nvme", Some('n'), Some('p')),
        numbered("mmcblk", None, Some('p')),
        numbered("disk", None, Some('s')),
        numbered("rdisk", None, Some('s')),
        numbered("md", None, None),
    ]
    .contains(&Some(""))
}

/// `text` after a number of 1 to 3 digits at its start.
fn number(text: &str) -> Option<&str> {
    run(text, |c| c.is_ascii_digit(), 1, 3)
}

/// `text` after its leading run of `min` to `max` characters matching `class` (the longest).
fn run(text: &str, class: impl Fn(char) -> bool, min: usize, max: usize) -> Option<&str> {
    let count = text.chars().take(max).take_while(|&c| class(c)).count();
    // The run is ASCII, so its length in bytes is its count.
    (count >= min).then(|| text.get(count..)).flatten()
}

#[cfg(test)]
mod tests;
