//! Path globs for repository-relative, `/`-separated paths.
//!
//! `**` as a whole segment matches zero or more directories, `*` any run of characters within one segment, `?` one character;
//! everything else is literal. Matching ignores ASCII letter case, as the guard's protected
//! paths do (APFS and NTFS are case-insensitive by default). A pattern matches the whole path
//! from the repository root, so `**/` is how it matches at any depth.

/// The longest pattern accepted, in bytes (H-13).
pub(crate) const MAX_GLOB_BYTES: usize = 256;

/// Why a pattern was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GlobError {
    /// The pattern is empty.
    #[error("the pattern is empty")]
    Empty,
    /// The pattern starts with `/`; paths are relative to the repository root.
    #[error("the pattern starts with `/`; patterns are relative to the repository root")]
    Absolute,
    /// The pattern holds a `\`; paths are `/`-separated on every platform.
    #[error("the pattern holds `\\`; separate directories with `/`")]
    Backslash,
    /// The pattern is longer than `MAX_GLOB_BYTES` (256).
    #[error("the pattern is longer than {MAX_GLOB_BYTES} bytes")]
    TooLong,
}

/// One segment of a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    /// `**`: zero or more whole segments.
    AnyDepth,
    /// A segment with `*` and `?`, lower-cased.
    Part(Vec<char>),
}

/// A compiled pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glob {
    pattern: String,
    segments: Vec<Segment>,
}

impl Glob {
    /// Compiles `pattern`.
    ///
    /// # Errors
    /// The [`GlobError`] naming why the pattern was refused.
    pub(crate) fn new(pattern: &str) -> Result<Self, GlobError> {
        if pattern.is_empty() {
            return Err(GlobError::Empty);
        }
        if pattern.len() > MAX_GLOB_BYTES {
            return Err(GlobError::TooLong);
        }
        if pattern.starts_with('/') {
            return Err(GlobError::Absolute);
        }
        if pattern.contains('\\') {
            return Err(GlobError::Backslash);
        }
        let segments = pattern
            .split('/')
            .map(|segment| match segment {
                "**" => Segment::AnyDepth,
                part => Segment::Part(part.chars().map(|c| c.to_ascii_lowercase()).collect()),
            })
            .collect();
        Ok(Self {
            pattern: pattern.to_owned(),
            segments,
        })
    }

    /// The pattern as written.
    pub(crate) fn as_str(&self) -> &str {
        &self.pattern
    }

    /// Whether `path` (relative to the repository root, `/`-separated) matches the whole
    /// pattern.
    pub(crate) fn matches(&self, path: &str) -> bool {
        let parts: Vec<Vec<char>> = path
            .split('/')
            .map(|part| part.chars().map(|c| c.to_ascii_lowercase()).collect())
            .collect();
        // reached[j]: the segments read so far match exactly the first j parts.
        let mut reached = vec![false; parts.len().saturating_add(1)];
        if let Some(first) = reached.first_mut() {
            *first = true;
        }
        for segment in &self.segments {
            reached = match segment {
                Segment::AnyDepth => {
                    let mut seen = false;
                    reached
                        .iter()
                        .map(|&here| {
                            seen |= here;
                            seen
                        })
                        .collect()
                }
                Segment::Part(pattern) => {
                    let mut next = vec![false; reached.len()];
                    for ((&here, part), slot) in
                        reached.iter().zip(&parts).zip(next.iter_mut().skip(1))
                    {
                        *slot = here && segment_matches(pattern, part);
                    }
                    next
                }
            };
        }
        reached.last().copied().unwrap_or(false)
    }
}

/// Whether one path segment `text` matches the segment `pattern` (`*` any run, `?` one
/// character), both lower-cased. Greedy with one backtrack point: linear in practice,
/// `O(pattern * text)` at worst.
fn segment_matches(pattern: &[char], text: &[char]) -> bool {
    let (mut p, mut t) = (0_usize, 0_usize);
    // The `*` last seen, and the text position it was tried at.
    let mut star: Option<(usize, usize)> = None;
    while let Some(&c) = text.get(t) {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, t));
                p = p.saturating_add(1);
            }
            Some(&want) if want == '?' || want == c => {
                p = p.saturating_add(1);
                t = t.saturating_add(1);
            }
            _ => match star {
                Some((star_p, star_t)) => {
                    let retry = star_t.saturating_add(1);
                    star = Some((star_p, retry));
                    p = star_p.saturating_add(1);
                    t = retry;
                }
                None => return false,
            },
        }
    }
    pattern
        .get(p..)
        .is_some_and(|rest| rest.iter().all(|&c| c == '*'))
}

#[cfg(test)]
mod tests;
