//! Which paths are lint or hook config, matched on their components after `..` is resolved
//! lexically, and where relative paths start once a `cd` has run.

use std::borrow::Cow;
use std::iter;

use crate::hook::guard::denylist::wrappers::skip_options;
use crate::hook::guard::shell::{EXTGLOB_OPENERS, Shape, Word};

/// The most components an extended glob span is read as beyond one (see `without_extglobs`).
const MAX_SPANNED: usize = 32;

/// A path component's name.
#[derive(Debug, Clone, Copy)]
enum Name {
    Exact(&'static str),
    /// Starts with `prefix` and ends with `suffix`; a glob is tried against `samples`.
    Affix {
        prefix: &'static str,
        suffix: &'static str,
        samples: &'static [&'static str],
    },
}

impl Name {
    /// Whether `component` is this name; when `glob` and the component holds a wildcard,
    /// whether it could match it. Letter case is ignored: on case-insensitive file systems
    /// (APFS, NTFS by default) `.CLAUDE/settings.json` is `.claude/settings.json`.
    fn matches(self, component: &str, glob: bool) -> bool {
        let wildcard = glob && component.contains(['*', '?', '[']);
        // The names are lower case; the component is lowered to compare.
        let component = component.to_ascii_lowercase();
        match self {
            Self::Exact(name) if wildcard => glob_match(&component, name),
            Self::Exact(name) => component == name,
            Self::Affix { samples, .. } if wildcard => {
                samples.iter().any(|sample| glob_match(&component, sample))
            }
            Self::Affix { prefix, suffix, .. } => component
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.ends_with(suffix)),
        }
    }
}

/// A protected path: a component, directly under `parent` when that is set.
#[derive(Debug)]
struct Protected {
    /// How a denial names it.
    label: &'static str,
    parent: Option<&'static str>,
    name: Name,
}

impl Protected {
    fn matches(&self, path: &Resolved<'_>, glob: bool) -> bool {
        let components = path.components.as_slice();
        match self.parent {
            None => components.iter().any(|c| self.name.matches(c, glob)),
            Some(parent) => {
                let under_parent = components.windows(2).any(|pair| {
                    matches!(pair, [above, component]
                        if Name::Exact(parent).matches(above, glob)
                            && self.name.matches(component, glob))
                });
                // Above the start, the parent is unknown and may be the protected one.
                let under_unknown = path.escaped
                    && components
                        .first()
                        .is_some_and(|first| self.name.matches(first, glob));
                under_parent || under_unknown
            }
        }
    }
}

/// A file protected wherever it is.
const fn file(name: &'static str) -> Protected {
    Protected {
        label: name,
        parent: None,
        name: Name::Exact(name),
    }
}

/// The protected paths.
const PROTECTED: [Protected; 13] = [
    Protected {
        label: "hooks/",
        parent: None,
        name: Name::Exact("hooks"),
    },
    Protected {
        label: ".claude/settings*.json",
        parent: Some(".claude"),
        name: Name::Affix {
            prefix: "settings",
            suffix: ".json",
            samples: &["settings.json", "settings.local.json"],
        },
    },
    Protected {
        label: ".cargo/config*",
        parent: Some(".cargo"),
        name: Name::Affix {
            prefix: "config",
            suffix: "",
            samples: &["config", "config.toml"],
        },
    },
    Protected {
        label: ".github/workflows/",
        parent: Some(".github"),
        name: Name::Exact("workflows"),
    },
    Protected {
        label: ".eslintrc*",
        parent: None,
        name: Name::Affix {
            prefix: ".eslintrc",
            suffix: "",
            samples: &[
                ".eslintrc",
                ".eslintrc.json",
                ".eslintrc.js",
                ".eslintrc.cjs",
                ".eslintrc.yml",
            ],
        },
    },
    file("clippy.toml"),
    file(".clippy.toml"),
    file("rustfmt.toml"),
    file(".rustfmt.toml"),
    file("deny.toml"),
    file(".pre-commit-config.yaml"),
    file("lefthook.yml"),
    // The gates' own settings: the orchestrate leads the brief hook trusts, the merge gate's
    // review switch and sensitive globs, `check_fast`.
    Protected {
        label: ".anneal/config.json",
        parent: Some(".anneal"),
        name: Name::Exact("config.json"),
    },
];

/// Directories that hold protected config: removing, moving or re-permissioning one changes it.
pub(super) const CONTAINERS: [(&str, &str); 4] = [
    (".anneal", ".anneal/"),
    (".claude", ".claude/"),
    (".github", ".github/"),
    (".cargo", ".cargo/"),
];

/// How a denial names a directory above the start, which may hold any of [`CONTAINERS`].
const ABOVE: &str = "a directory above the working directory, which may hold .claude/";

/// Where relative paths start, as far as the `cd` commands seen so far say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Cwd {
    /// No `cd` yet: the hook's working directory.
    Start,
    /// A directory a `cd` named (relative to the start, or absolute), and its shape.
    Dir(String, Shape),
    /// A `cd` to a variable or to `-` (the previous directory): anywhere.
    Unknown,
}

impl Cwd {
    /// Follows `cd <args>` (or `pushd`); with no directory it goes home, `~`. The directory is
    /// copied: it outlives the command.
    pub(crate) fn change(&mut self, args: &[Word]) {
        let Some(dir) = skip_options(args, &[]).first() else {
            *self = Self::Dir("~".to_owned(), Shape::Pattern);
            return;
        };
        // An unquoted leading `~` is the home directory, which is absolute.
        let rooted = is_absolute(&dir.text) || (dir.text.starts_with('~') && !dir.is_literal());
        *self = match &*self {
            _ if dir.shape == Shape::Dynamic || dir.text == "-" => Self::Unknown,
            Self::Dir(base, shape) if !rooted => {
                Self::Dir(format!("{base}/{}", dir.text), dir.shape.max(*shape))
            }
            Self::Unknown if !rooted => Self::Unknown,
            Self::Start | Self::Dir(..) | Self::Unknown => Self::Dir(dir.text.clone(), dir.shape),
        };
    }
}

/// A path's components after `.` and `..` are resolved, and whether `..` climbed above where
/// the path starts.
#[derive(Debug)]
struct Resolved<'a> {
    components: Vec<&'a str>,
    escaped: bool,
}

fn is_absolute(path: &str) -> bool {
    path.starts_with(['/', '\\']) || path.get(1..2) == Some(":")
}

/// `path` resolved lexically; `escaped` from the start when relative paths may be anywhere.
fn resolve(path: &str, escaped: bool) -> Resolved<'_> {
    let absolute = is_absolute(path);
    let mut resolved = Resolved {
        components: Vec::new(),
        escaped: escaped && !absolute,
    };
    for component in path.split(['/', '\\']) {
        match component {
            "" | "." => {}
            ".." => {
                if resolved.components.pop().is_none() && !absolute {
                    resolved.escaped = true;
                }
            }
            name => resolved.components.push(name),
        }
    }
    resolved
}

/// The last component of `path` once resolved.
pub(super) fn last_component(path: &str) -> Option<&str> {
    resolve(path, false).components.last().copied()
}

/// One way to read a path: its text, whether it is a glob, and whether a relative path may
/// start anywhere.
#[derive(Debug)]
struct Reading<'a> {
    path: Cow<'a, str>,
    glob: bool,
    anywhere: bool,
}

/// The ways `path` may be read from `cwd`: as written (from anywhere after a `cd` the guard
/// cannot follow), and joined to the directory a `cd` named.
fn readings<'a>(path: &'a str, shape: Shape, cwd: &Cwd) -> impl Iterator<Item = Reading<'a>> {
    let relative = !is_absolute(path);
    let glob = shape != Shape::Literal;
    let anywhere = relative && *cwd == Cwd::Unknown;
    let as_written = Reading {
        path: Cow::Borrowed(path),
        glob,
        anywhere,
    };
    let joined = match cwd {
        Cwd::Dir(dir, dir_shape) if relative => Some(Reading {
            path: Cow::Owned(format!("{dir}/{path}")),
            glob: glob || *dir_shape != Shape::Literal,
            anywhere: false,
        }),
        Cwd::Start | Cwd::Dir(..) | Cwd::Unknown => None,
    };
    iter::once(as_written)
        .chain(joined)
        .flat_map(Reading::without_extglobs)
}

impl Reading<'_> {
    /// This reading, or for a glob each reading with its extended glob groups cut out before
    /// the path is split: a `/` inside a group is no separator (`.clau@(de|x/y)` is `.claude`).
    fn without_extglobs(self) -> impl Iterator<Item = Self> {
        let cut = if self.glob {
            without_extglobs(&self.path)
        } else {
            None
        };
        let (glob, anywhere) = (self.glob, self.anywhere);
        let whole = cut.is_none().then_some(self);
        whole
            .into_iter()
            .chain(cut.into_iter().flatten().map(move |path| Reading {
                path: Cow::Owned(path),
                glob,
                anywhere,
            }))
    }
}

/// The protected path writing `path` (seen from `cwd`) changes, if any.
pub(super) fn protected(path: &str, shape: Shape, cwd: &Cwd) -> Option<&'static str> {
    readings(path, shape, cwd).find_map(|reading| {
        let resolved = resolve(&reading.path, reading.anywhere);
        PROTECTED
            .iter()
            .find(|protected| protected.matches(&resolved, reading.glob))
            .map(|protected| protected.label)
    })
}

/// The protected path removing (or re-permissioning) `path` changes, if any: a protected path,
/// a directory holding one, or a directory above the start.
pub(super) fn contains_protected(path: &str, shape: Shape, cwd: &Cwd) -> Option<&'static str> {
    protected(path, shape, cwd).or_else(|| {
        readings(path, shape, cwd).find_map(|reading| {
            let resolved = resolve(&reading.path, reading.anywhere);
            let Some(last) = resolved.components.last() else {
                return resolved.escaped.then_some(ABOVE);
            };
            CONTAINERS
                .iter()
                .find(|(name, _)| Name::Exact(name).matches(last, reading.glob))
                .map(|(_, label)| *label)
        })
    })
}

/// The ways `path` may be read with its extended glob groups (`@(de)`, `!(x)`, `?(a|b)`)
/// cut out: the span from its first opener to its last `)` (or to its end when no `)`
/// follows) becomes one, two or more `*` components, up to one more than the `/`s it holds.
///
/// Quoting is not known here, so a quoted `)` could seem to close a group early
/// (`.c@(laude|l")")`), and a `/` inside a group (`.clau@(de|x/y)`) is no separator, as bash
/// matches a group within one component; the span's real `)` is never after its last one,
/// and it holds at most that many components, each of which a `*` matches, so the readings
/// only widen a match. The count is capped at [`MAX_SPANNED`]: past a few, more `*`s change a
/// match only through as many `..` after the span.
fn without_extglobs(path: &str) -> Option<Vec<String>> {
    let start = path
        .char_indices()
        .zip(path.chars().skip(1))
        .find(|((_, c), next)| EXTGLOB_OPENERS.contains(c) && *next == '(')
        .map(|((at, _), _)| at)?;
    let close = path.rfind(')').filter(|close| *close > start);
    let after = close
        .and_then(|close| path.get(close.saturating_add(1)..))
        .unwrap_or("");
    let span = close
        .and_then(|close| path.get(start..close))
        .or_else(|| path.get(start..))
        .unwrap_or("");
    let before = path.get(..start).unwrap_or("");
    let slashes = span.matches(['/', '\\']).count().min(MAX_SPANNED);
    Some(
        (0..=slashes)
            .map(|extra| format!("{before}*{}{after}", "/*".repeat(extra)))
            .collect(),
    )
}

/// Whether `text` matches the shell pattern `pattern` (`*`, `?`, and `[...]` as any one
/// character, which can only widen a match).
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, t));
                p += 1;
            }
            Some('[') => {
                let close = pattern.iter().skip(p).position(|c| *c == ']');
                p = close.map_or(p + 1, |close| p + close + 1);
                t += 1;
            }
            Some(c) if *c == '?' || Some(c) == text.get(t) => {
                p += 1;
                t += 1;
            }
            _ => {
                let Some((star_p, star_t)) = star else {
                    return false;
                };
                p = star_p + 1;
                t = star_t + 1;
                star = Some((star_p, t));
            }
        }
    }
    pattern.iter().skip(p).all(|c| *c == '*')
}
