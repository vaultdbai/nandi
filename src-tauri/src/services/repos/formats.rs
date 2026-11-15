//! Format versions of the gold copy's definition files.
//!
//! Each kind of definition file has a format version, written as the
//! top-level `format_version` key (in the frontmatter of a `skill.md`). A
//! file without one is at format 1. When a file format changes, the change
//! is one small pure function from version N to N+1, appended to that
//! kind's `steps`:
//!
//! - **Reading** ([`FileFormat::read`]): an older file is upgraded in memory,
//!   so it always loads. A file from a newer Nandi is refused and left as it
//!   is.
//! - **At startup** ([`super::Repos::upgrade_formats`]): older files are
//!   written back at the current format and committed together, so the old
//!   text stays in git history.
//! - **Writing** ([`FileFormat::stamp`]): what the app saves carries the
//!   current version (nothing while it is 1, so today's files don't change).
//!
//! The domain types never see `format_version`: it is taken off before they
//! parse a file. See `docs/developer/data-model.md#file-format-versions`.

use std::borrow::Cow;
use std::fmt;
use std::path::{Path, PathBuf};

use serde_yaml::{Mapping, Value};

use crate::services::domain::agent::split_frontmatter;

/// The key that holds a file's format version.
pub const VERSION_KEY: &str = "format_version";

/// One upgrade, from version N to N+1, of a file's YAML mapping.
pub type Step = fn(&mut Mapping) -> Result<(), String>;

/// Where a file keeps its YAML.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Syntax {
    /// The whole file is YAML
    Yaml,
    /// Markdown with `---` frontmatter; the body is never touched
    Frontmatter,
}

/// A kind of definition file and how its format moves forward.
#[derive(Debug)]
pub struct FileFormat {
    /// What one file is, for messages ("workflow")
    pub noun: &'static str,
    pub plural: &'static str,
    pub syntax: Syntax,
    /// `steps[0]` upgrades format 1 to 2, `steps[1]` 2 to 3, ...
    pub steps: &'static [Step],
}

/// An agent's `skill.md` (its frontmatter)
pub const AGENT: FileFormat = FileFormat::new("agent", "agents", Syntax::Frontmatter, &[]);
/// A workflow's `.yaml`, global or agent-scoped
pub const WORKFLOW: FileFormat = FileFormat::new("workflow", "workflows", Syntax::Yaml, &[]);
/// A trigger's `.yaml`
pub const TRIGGER: FileFormat = FileFormat::new("trigger", "triggers", Syntax::Yaml, &[]);
/// `.nandi/settings.yaml`
pub const SETTINGS: FileFormat =
    FileFormat::new("settings file", "settings files", Syntax::Yaml, &[]);
/// `.nandi/connectors.yaml`
pub const CONNECTORS: FileFormat =
    FileFormat::new("connectors file", "connectors files", Syntax::Yaml, &[]);

/// Why a file's format can't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    /// Written by a newer Nandi: refused, never rewritten
    Newer { found: u32, current: u32 },
    /// The version key or the YAML around it is broken
    Invalid(String),
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Newer { found, current } => write!(
                f,
                "this file was written by a newer version of Nandi (format {found}; this \
                 version reads up to {current}). It is left as it is. Update Nandi to use it."
            ),
            Self::Invalid(reason) => write!(f, "{VERSION_KEY}: {reason}"),
        }
    }
}

/// A file's text upgraded to the current format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upgraded {
    pub from: u32,
    pub text: String,
}

impl FileFormat {
    pub const fn new(
        noun: &'static str,
        plural: &'static str,
        syntax: Syntax,
        steps: &'static [Step],
    ) -> Self {
        Self {
            noun,
            plural,
            syntax,
            steps,
        }
    }

    /// The format this build writes.
    pub const fn current(&self) -> u32 {
        self.steps.len() as u32 + 1
    }

    /// The text for the domain to parse: at the current format, without
    /// the version key. A file already like that is returned as it is.
    pub fn read<'a>(&self, text: &'a str) -> Result<Cow<'a, str>, FormatError> {
        if !text.contains(VERSION_KEY) && self.current() == 1 {
            return Ok(Cow::Borrowed(text));
        }
        let mut document = Document::parse(text, self.syntax)?;
        let found = document.take_version()?;
        self.upgrade_from(found, &mut document.yaml)?;
        Ok(Cow::Owned(document.render(None)))
    }

    /// The file's text at the current format, stamped with it, or `None`
    /// when it already is. This is what a file at an older format is
    /// written back as.
    pub fn upgrade(&self, text: &str) -> Result<Option<Upgraded>, FormatError> {
        let mut document = Document::parse(text, self.syntax)?;
        let found = document.take_version()?;
        if found == self.current() {
            return Ok(None);
        }
        self.upgrade_from(found, &mut document.yaml)?;
        Ok(Some(Upgraded {
            from: found,
            text: document.render(Some(self.current())),
        }))
    }

    /// Text the app serialized (at the current format) as it is saved:
    /// stamped with the current version. Unchanged while that is 1.
    pub fn stamp<'a>(&self, text: &'a str) -> Cow<'a, str> {
        if self.current() == 1 {
            return Cow::Borrowed(text);
        }
        let line = format!("{VERSION_KEY}: {}\n", self.current());
        Cow::Owned(match self.syntax {
            Syntax::Yaml => format!("{line}{text}"),
            Syntax::Frontmatter => match text.strip_prefix("---\n") {
                Some(rest) => format!("---\n{line}{rest}"),
                None => format!("---\n{line}---\n{text}"),
            },
        })
    }

    /// Text a person wrote (missing version = format 1), as it is saved:
    /// upgraded and stamped when older, unchanged when current.
    pub fn canonical<'a>(&self, text: &'a str) -> Result<Cow<'a, str>, FormatError> {
        if !text.contains(VERSION_KEY) && self.current() == 1 {
            return Ok(Cow::Borrowed(text));
        }
        Ok(match self.upgrade(text)? {
            Some(upgraded) => Cow::Owned(upgraded.text),
            None => Cow::Borrowed(text),
        })
    }

    fn upgrade_from(&self, found: u32, yaml: &mut Mapping) -> Result<(), FormatError> {
        let current = self.current();
        if found > current {
            return Err(FormatError::Newer { found, current });
        }
        for (index, step) in self.steps.iter().enumerate().skip(found as usize - 1) {
            step(yaml).map_err(|e| {
                FormatError::Invalid(format!("upgrading to format {}: {e}", index + 2))
            })?;
        }
        Ok(())
    }
}

/// What one startup pass wrote back ([`upgrade_files`]).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FormatUpgrade {
    /// The files written back, in the order they were found
    pub paths: Vec<PathBuf>,
    /// How many files of each kind, for the commit message
    kinds: Vec<KindCount>,
}

#[derive(Debug, PartialEq, Eq)]
struct KindCount {
    noun: &'static str,
    plural: &'static str,
    to: u32,
    files: usize,
}

impl FormatUpgrade {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// "Upgrade 3 workflows to format 2", with each kind it touched.
    pub fn commit_message(&self) -> String {
        let parts: Vec<String> = self
            .kinds
            .iter()
            .map(|kind| {
                let what = if kind.files == 1 {
                    kind.noun
                } else {
                    kind.plural
                };
                format!("{} {what} to format {}", kind.files, kind.to)
            })
            .collect();
        format!("Upgrade {}", parts.join(", "))
    }

    fn add(&mut self, format: &FileFormat, path: PathBuf) {
        self.paths.push(path);
        match self.kinds.iter_mut().find(|kind| kind.noun == format.noun) {
            Some(kind) => kind.files += 1,
            None => self.kinds.push(KindCount {
                noun: format.noun,
                plural: format.plural,
                to: format.current(),
                files: 1,
            }),
        }
    }
}

/// Writes each file at an older format back at the current one, through
/// `write`. A file from a newer Nandi, or one that can't be read, is left
/// alone: loading it reports why.
pub fn upgrade_files(
    files: &[(&FileFormat, PathBuf)],
    mut write: impl FnMut(&Path, &str) -> std::io::Result<()>,
) -> FormatUpgrade {
    let mut done = FormatUpgrade::default();
    for (format, path) in files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        match format.upgrade(&text) {
            Ok(None) => {}
            Ok(Some(upgraded)) => match write(path, &upgraded.text) {
                Ok(()) => {
                    log::info!(
                        "Upgraded {} from format {} to {}",
                        path.display(),
                        upgraded.from,
                        format.current()
                    );
                    done.add(format, path.clone());
                }
                Err(e) => log::error!("Failed to upgrade {}: {e}", path.display()),
            },
            Err(e) => log::warn!("Not upgrading {}: {e}", path.display()),
        }
    }
    done
}

/// A file split into its YAML mapping and, for Markdown, its body.
struct Document<'a> {
    syntax: Syntax,
    yaml: Mapping,
    body: &'a str,
}

impl<'a> Document<'a> {
    fn parse(text: &'a str, syntax: Syntax) -> Result<Self, FormatError> {
        let (yaml, body) = match syntax {
            Syntax::Yaml => (Some(text), ""),
            Syntax::Frontmatter => split_frontmatter(text).map_err(FormatError::Invalid)?,
        };
        let yaml = match yaml.map(serde_yaml::from_str::<Value>).transpose() {
            Ok(None | Some(Value::Null)) => Mapping::new(),
            Ok(Some(Value::Mapping(mapping))) => mapping,
            Ok(Some(_)) => return Err(FormatError::Invalid("the YAML isn't a mapping".into())),
            Err(e) => return Err(FormatError::Invalid(e.to_string())),
        };
        Ok(Self { syntax, yaml, body })
    }

    /// Removes the version key; a file without one is at format 1.
    fn take_version(&mut self) -> Result<u32, FormatError> {
        match self.yaml.remove(VERSION_KEY) {
            None => Ok(1),
            Some(Value::Number(n)) => n
                .as_u64()
                .filter(|v| *v >= 1)
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| FormatError::Invalid(format!("{n} isn't a format version"))),
            Some(other) => Err(FormatError::Invalid(format!(
                "expected a whole number, found {other:?}"
            ))),
        }
    }

    /// The file's text, with `version` first in the YAML when given.
    fn render(&self, version: Option<u32>) -> String {
        let mut yaml = Mapping::new();
        if let Some(version) = version {
            yaml.insert(VERSION_KEY.into(), version.into());
        }
        yaml.extend(self.yaml.clone());
        let yaml = if yaml.is_empty() {
            String::new()
        } else {
            serde_yaml::to_string(&yaml).unwrap_or_default()
        };
        match self.syntax {
            Syntax::Yaml => yaml,
            Syntax::Frontmatter => format!("---\n{yaml}---\n{}", self.body),
        }
    }
}

#[cfg(test)]
#[path = "formats_tests.rs"]
mod tests;
