//! The local markdown tracker: one ticket per `<dir>/*.md` file.
//!
//! A ticket's id is its file stem (`003-auth.md` is `003-auth`); `README.md` (any case) is not a
//! ticket, nor is anything that is not a regular `.md` file. The optional frontmatter is the
//! same YAML subset `SKILL.md` uses (`lint::skills::frontmatter`), with these keys:
//!
//! - `blocked_by`: ids that must be done first, as a flow list (`[001, "002"]`), a block list
//!   (`- 001` lines) or a comma list (`001, 002`);
//! - `status`: `done` or `closed` (any case) is done, anything else open;
//! - `title`: else the first `# ` heading, else the id;
//! - `files`: files the ticket will touch, in the same list forms (for the estimate).
//!
//! Everything after the frontmatter is the body. Other keys are ignored.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::dag::ticket::{Status, Ticket, TicketId};
use crate::dag::tracker::{MAX_TICKETS, Tracker, TrackerError};
use crate::lint::skills::frontmatter::{self, Document};

/// The largest ticket file read (H-13).
pub const MAX_TICKET_BYTES: u64 = 1024 * 1024;

/// Why the markdown tickets could not be read.
#[derive(Debug, thiserror::Error)]
pub enum MarkdownError {
    /// The tickets directory could not be listed.
    #[error("listing tickets in {}", dir.display())]
    ReadDir {
        /// The directory.
        dir: PathBuf,
        /// The I/O failure.
        #[source]
        source: io::Error,
    },
    /// A ticket file could not be read.
    #[error("reading {}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// The I/O failure.
        #[source]
        source: io::Error,
    },
    /// A ticket file is larger than [`MAX_TICKET_BYTES`].
    #[error("{} is {size} bytes, over the {MAX_TICKET_BYTES}-byte limit", path.display())]
    TooLarge {
        /// The file.
        path: PathBuf,
        /// Its size.
        size: u64,
    },
    /// A ticket file is not UTF-8, or its name is not.
    #[error("{} is not UTF-8", path.display())]
    NotUtf8 {
        /// The file.
        path: PathBuf,
    },
    /// A ticket's frontmatter could not be read.
    #[error("{}:{line}: {reason}", path.display())]
    Ticket {
        /// The file.
        path: PathBuf,
        /// 1-based line.
        line: usize,
        /// What was wrong.
        reason: &'static str,
    },
    /// The directory holds more than [`MAX_TICKETS`] tickets.
    #[error("{} holds more than {MAX_TICKETS} tickets", dir.display())]
    TooMany {
        /// The directory.
        dir: PathBuf,
    },
}

/// The tickets in one directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Markdown {
    dir: PathBuf,
}

impl Markdown {
    /// The tracker over `dir` (`tickets` by default on the command line).
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Tracker for Markdown {
    fn name(&self) -> &'static str {
        "markdown"
    }

    fn read(&self) -> Result<Vec<Ticket>, TrackerError> {
        let files = ticket_files(&self.dir)?;
        let mut tickets = Vec::with_capacity(files.len());
        for (id, path) in files {
            let text = read_ticket(&path)?;
            let ticket = parse_ticket(&id, &text)
                .map_err(|BadLine(line, reason)| MarkdownError::Ticket { path, line, reason })?;
            tickets.push(ticket);
        }
        Ok(tickets)
    }
}

/// `(id, path)` of each ticket file in `dir`, sorted by file name so the first error is the
/// same on every run.
fn ticket_files(dir: &Path) -> Result<Vec<(String, PathBuf)>, MarkdownError> {
    let read_dir_error = |source| MarkdownError::ReadDir {
        dir: dir.to_path_buf(),
        source,
    };
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).map_err(read_dir_error)? {
        let path = entry.map_err(read_dir_error)?.path();
        let is_md = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
        if !is_md {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            return Err(MarkdownError::NotUtf8 { path });
        };
        if stem.eq_ignore_ascii_case("readme") {
            continue;
        }
        let stem = stem.to_owned();
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(source) => return Err(MarkdownError::Read { path, source }),
        };
        if !metadata.is_file() {
            continue;
        }
        if files.len() == MAX_TICKETS {
            return Err(MarkdownError::TooMany {
                dir: dir.to_path_buf(),
            });
        }
        files.push((stem, path));
    }
    files.sort();
    Ok(files)
}

/// The text of the ticket at `path`, at most [`MAX_TICKET_BYTES`].
fn read_ticket(path: &Path) -> Result<String, MarkdownError> {
    let read_error = |source| MarkdownError::Read {
        path: path.to_path_buf(),
        source,
    };
    let file = File::open(path).map_err(read_error)?;
    let size = file.metadata().map_err(read_error)?.len();
    if size > MAX_TICKET_BYTES {
        return Err(MarkdownError::TooLarge {
            path: path.to_path_buf(),
            size,
        });
    }
    let mut bytes = Vec::new();
    // One byte past the cap tells a file that grew since its size was read.
    file.take(MAX_TICKET_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(read_error)?;
    let read = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if read > MAX_TICKET_BYTES {
        return Err(MarkdownError::TooLarge {
            path: path.to_path_buf(),
            size: read,
        });
    }
    String::from_utf8(bytes).map_err(|_not_utf8| MarkdownError::NotUtf8 {
        path: path.to_path_buf(),
    })
}

/// A line of a ticket that could not be read, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BadLine(usize, &'static str);

/// Reads one ticket's text; `id` is its file stem.
fn parse_ticket(id: &str, text: &str) -> Result<Ticket, BadLine> {
    let starts_with_fence = text.lines().next().is_some_and(|l| l.trim_end() == "---");
    if !starts_with_fence {
        return Ok(Ticket {
            id: TicketId::new(id),
            title: heading(text).unwrap_or(id).to_owned(),
            body: text.to_owned(),
            blocked_by: Vec::new(),
            files: Vec::new(),
            status: Status::Open,
        });
    }
    let doc = frontmatter::parse(text).map_err(|e| BadLine(e.line, e.reason))?;
    let title = match doc.value("title") {
        "" => heading(doc.body).unwrap_or(id),
        title => title,
    };
    let status = match doc.value("status").trim().to_ascii_lowercase().as_str() {
        "done" | "closed" => Status::Done,
        _ => Status::Open,
    };
    Ok(Ticket {
        id: TicketId::new(id),
        title: title.to_owned(),
        body: doc.body.to_owned(),
        blocked_by: list(&doc, "blocked_by")?
            .into_iter()
            .map(TicketId::new)
            .collect(),
        files: list(&doc, "files")?,
        status,
    })
}

/// The text of the first `# ` heading.
fn heading(body: &str) -> Option<&str> {
    body.lines()
        .find_map(|line| line.strip_prefix("# "))
        .map(str::trim)
}

/// The items of list field `key`: a flow list, a block list or a comma list; none when absent.
fn list(doc: &Document<'_>, key: &str) -> Result<Vec<String>, BadLine> {
    let Some(field) = doc.field(key) else {
        return Ok(Vec::new());
    };
    let line = field.line;
    let value = field.value.trim();
    let items: Vec<&str> = if let Some(inner) = value.strip_prefix('[') {
        let inner = inner
            .strip_suffix(']')
            .ok_or(BadLine(line, "unclosed `[` list"))?;
        inner.split(',').collect()
    } else if value.starts_with('-') {
        value
            .lines()
            .map(|item| {
                item.strip_prefix("- ")
                    .or_else(|| item.strip_prefix('-').filter(|rest| rest.is_empty()))
                    .ok_or(BadLine(line, "a block list item must be `- item`"))
            })
            .collect::<Result<_, _>>()?
    } else {
        value.split(',').collect()
    };
    items
        .into_iter()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| frontmatter::unquote(item, line).map_err(|e| BadLine(e.line, e.reason)))
        .collect()
}

#[cfg(test)]
mod tests;
