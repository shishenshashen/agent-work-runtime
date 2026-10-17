use crate::{Manifest, SourceSpec};
use awr_core::{Error, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
    process::Command,
};
use url::Url;

#[derive(Debug, Clone)]
pub enum Locator {
    /// Already-authorized canonical path. Use `from_spec` for configured sources;
    /// reads deliberately refuse links substituted after that authority check.
    File(PathBuf),
    Git {
        revision: String,
        path: PathBuf,
    },
    /// Project-root-relative path; identity is `project:///<relative-path>`.
    Project {
        path: PathBuf,
    },
}

#[derive(Debug, Clone)]
pub struct SourceSnapshot {
    pub locator: String,
    pub fingerprint: String,
    pub bytes: Vec<u8>,
    /// Byte-bound source proof; every use rechecks the retained snapshot.
    pub content_review: Option<awr_core::VerifiedSourceContentReview>,
}
impl SourceSnapshot {
    /// Validate retained source bytes without reading the original file or Git repository.
    /// Git fingerprints bind the resolved locator as well as the blob content.
    pub fn verify_fingerprint(&self) -> bool {
        let expected = if let Some(git) = self.locator.strip_prefix("git://") {
            let Some((commit, path)) = git.split_once(':') else {
                return false;
            };
            if ![40, 64].contains(&commit.len())
                || !commit.bytes().all(|b| b.is_ascii_hexdigit())
                || relative(Path::new(path)).is_err()
            {
                return false;
            }
            let mut hash = Sha256::new();
            hash.update(self.locator.as_bytes());
            hash.update([0]);
            hash.update(&self.bytes);
            format!("sha256:{:x}", hash.finalize())
        } else if self.locator.starts_with("file://") || self.locator.starts_with("project://") {
            fingerprint(&self.bytes)
        } else {
            return false;
        };
        self.fingerprint == expected
    }

    pub fn text(&self) -> Result<&str> {
        self.validate_content()?;
        std::str::from_utf8(&self.bytes)
            .map_err(|e| Error::InvalidInput(format!("source must be UTF-8: {e}")))
    }

    pub fn validate_content(&self) -> Result<()> {
        match &self.content_review {
            Some(review) => {
                if !self.verify_fingerprint() {
                    return Err(Error::SourceConflict(
                        "reviewed snapshot fingerprint is inconsistent".into(),
                    ));
                }
                review.verify_bytes(&self.bytes, &self.locator)
            }
            None => awr_core::ensure_public_source(&self.bytes, &self.locator),
        }
    }
    pub fn ensure_value(&self, value: &serde_json::Value) -> Result<()> {
        self.validate_content()?;
        match &self.content_review {
            Some(review) => review.ensure_value(value),
            None => awr_core::ensure_public_value(value),
        }
    }

    /// One-based inclusive line range; preserve the original newline bytes.
    pub fn section_text(&self, start: usize, end: usize) -> Result<&str> {
        let text = self.text()?;
        let lines: Vec<_> = text.split_inclusive('\n').collect();
        if start == 0 || end < start || end > lines.len() {
            return Err(Error::InvalidInput(
                "section line range is outside the source".into(),
            ));
        }
        let offset: usize = lines[..start - 1].iter().map(|s| s.len()).sum();
        let size: usize = lines[start - 1..end].iter().map(|s| s.len()).sum();
        Ok(&text[offset..offset + size])
    }

    pub fn section_fingerprint(&self, start: usize, end: usize) -> Result<String> {
        Ok(fingerprint(self.section_text(start, end)?.as_bytes()))
    }
}

pub fn read_capped(path: &Path, cap: u64) -> Result<Vec<u8>> {
    let file = File::open(path)
        .map_err(|e| Error::SourceUnavailable(format!("{}: {e}", path.display())))?;
    let bytes = read_file_capped(file, path, cap)?;
    awr_core::ensure_public_source(&bytes, &path.to_string_lossy())?;
    Ok(bytes)
}

/// Read a canonical source path whose authority was checked by Locator/Manifest.
/// Unlike caller-supplied input files, it must not be redirected by a replacement link.
pub fn read_source_capped(path: &Path, cap: u64) -> Result<Vec<u8>> {
    let bytes = read_source_for_review(path, cap)?;
    awr_core::ensure_public_source(&bytes, &path.to_string_lossy())?;
    Ok(bytes)
}

/// Bounded diagnostic read. Callers must not archive its bytes before review.
pub fn read_source_for_review(path: &Path, cap: u64) -> Result<Vec<u8>> {
    read_file_capped(crate::open_file_exact(path)?, path, cap)
}

fn read_file_capped(file: File, _path: &Path, cap: u64) -> Result<Vec<u8>> {
    if cap == 0 {
        return Err(Error::InvalidInput("read cap must be positive".into()));
    }
    let read_limit = cap
        .checked_add(1)
        .ok_or_else(|| Error::InvalidInput("read cap is too large".into()))?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(Error::InvalidInput(
            "source reader needs a regular file".into(),
        ));
    }
    if metadata.len() > cap {
        return Err(Error::InvalidInput(format!(
            "source exceeds {cap} byte read cap"
        )));
    }
    let mut bytes = Vec::new();
    file.take(read_limit).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > cap {
        return Err(Error::InvalidInput(format!(
            "source exceeds {cap} byte read cap"
        )));
    }
    Ok(bytes)
}
pub fn fingerprint(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn relative(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(Error::RuleViolation(
            "Git source path must stay inside the project".into(),
        ));
    }
    Ok(())
}

/// Strip a `#L<start>[-L<end>]` selector; any other `#` stays part of the path.
fn strip_line_suffix(value: &str) -> Result<&str> {
    let Some(index) = value.find('#') else {
        return Ok(value);
    };
    let Some(range) = value[index + 1..].strip_prefix('L') else {
        return Ok(value);
    };
    let numeric = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    let mut parts = range.split('-');
    let start = parts.next().unwrap_or_default();
    let end = parts.next();
    if !numeric(start) || parts.next().is_some() {
        return Err(Error::InvalidInput(
            "line suffix must be #L<start> or #L<start>-L<end>".into(),
        ));
    }
    if let Some(end) = end {
        let Some(end) = end.strip_prefix('L') else {
            return Err(Error::InvalidInput(
                "line suffix must be #L<start> or #L<start>-L<end>".into(),
            ));
        };
        if !numeric(end) {
            return Err(Error::InvalidInput(
                "line suffix must be #L<start> or #L<start>-L<end>".into(),
            ));
        }
    }
    Ok(&value[..index])
}

/// RFC 3986 scheme: an ASCII letter followed by letters, digits, `+`, `.` or `-`.
fn is_scheme(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
}

impl Locator {
    /// Registration identity keeps a Git ref stable while snapshot locators bind immutable commits.
    pub fn identity(&self) -> Result<String> {
        match self {
            Self::File(path) => Url::from_file_path(path)
                .map(|url| url.to_string())
                .map_err(|_| Error::InvalidInput("invalid file locator".into())),
            Self::Git { revision, path } => {
                relative(path)?;
                let path =
                    path.components()
                        .filter_map(|part| match part {
                            Component::Normal(value) => Some(value.to_str().ok_or_else(|| {
                                Error::InvalidInput("Git path must be UTF-8".into())
                            })),
                            _ => None,
                        })
                        .collect::<Result<Vec<_>>>()?
                        .join("/");
                Ok(format!("git://{revision}:{path}"))
            }
            Self::Project { path } => {
                relative(path)?;
                let path = path
                    .components()
                    .filter_map(|part| match part {
                        Component::Normal(value) => Some(value.to_str().ok_or_else(|| {
                            Error::InvalidInput("project path must be UTF-8".into())
                        })),
                        _ => None,
                    })
                    .collect::<Result<Vec<_>>>()?
                    .join("/");
                Ok(format!("project:///{path}"))
            }
        }
    }
    pub fn from_spec(root: &Path, manifest: &Manifest, spec: &SourceSpec) -> Result<Self> {
        if let Some(path) = &spec.path {
            return Self::file(root, manifest, path);
        }
        let value = spec
            .locator
            .as_deref()
            .ok_or_else(|| Error::InvalidInput("missing locator".into()))?;
        // A manifest locator keeps the original strict rule: only the canonical
        // `file://` URL, the pinned `git://` form and the project-relative
        // `project:///` form are accepted, so no manifest that parsed before
        // changes meaning now that evidence locators are also normalized. Bare
        // paths stay the `path` field's job.
        if value.starts_with("file://")
            || value.starts_with("git://")
            || value.starts_with("project://")
        {
            return Self::parse(root, manifest, value);
        }
        Err(Error::Unsupported(
            "V1 supports file://, git:// and project:// sources".into(),
        ))
    }

    /// Parse one locator string through the single path authority.
    ///
    /// Accepted forms are the canonical `file://` URL produced by
    /// [`Locator::identity`], a pinned
    /// `git://<revision>:<project-relative-path>` locator, a
    /// `project:///<project-relative-path>` locator that resolves against the
    /// current checkout, and a bare filesystem path (the form `awr evidence add`
    /// has always accepted). A `#L<start>[-L<end>]` line selector is stripped
    /// before any of them is resolved, so it never enters identity or fingerprint.
    /// `file://`, `project:///` and bare paths are canonicalized and must stay
    /// inside the project root or a configured authorized root; Git paths must
    /// stay project-relative. Any other scheme is reported as unsupported before
    /// the filesystem is touched, so a remote reference never costs a read.
    pub fn parse(root: &Path, manifest: &Manifest, value: &str) -> Result<Self> {
        if value.trim().is_empty() {
            return Err(Error::InvalidInput("locator must not be empty".into()));
        }
        let value = strip_line_suffix(value)?;
        if value.starts_with("file://") {
            let url = Url::parse(value).map_err(|e| Error::InvalidInput(e.to_string()))?;
            if url.query().is_some() || url.fragment().is_some() {
                return Err(Error::InvalidInput(
                    "file locator must not contain query or fragment".into(),
                ));
            }
            let path = url.to_file_path().map_err(|_| {
                Error::InvalidInput("file locator must identify a local absolute path".into())
            })?;
            return Self::file(root, manifest, &path);
        }
        if let Some(value) = value.strip_prefix("git://") {
            let (revision, path) = value.split_once(':').ok_or_else(|| {
                Error::InvalidInput(
                    "Git locator syntax is git://<ref>:<project-relative-path>".into(),
                )
            })?;
            if revision.is_empty()
                || revision.starts_with('-')
                || revision.chars().any(char::is_whitespace)
            {
                return Err(Error::InvalidInput("invalid Git revision".into()));
            }
            let path = PathBuf::from(path);
            relative(&path)?;
            return Ok(Self::Git {
                revision: revision.into(),
                path,
            });
        }
        if let Some(value) = value.strip_prefix("project://") {
            let Some(path) = value.strip_prefix('/') else {
                return Err(Error::InvalidInput(
                    "project locator syntax is project:///<project-relative-path>".into(),
                ));
            };
            let path = PathBuf::from(path);
            relative(&path)?;
            // The same authority check as file:// sources: a project locator can
            // never point outside the project or its authorized roots.
            Self::file(root, manifest, &path)?;
            return Ok(Self::Project { path });
        }
        if value.contains("://") {
            return Err(Error::Unsupported(
                "V1 supports file://, git:// and project:// sources".into(),
            ));
        }
        // An URI-scheme prefix such as `node:`, `s3:` or a `file:`/`git:` spelling
        // missing its `//` is not a filesystem path. The scheme must be at least two
        // characters so a Windows drive letter (`C:\...`) keeps its bare-path
        // meaning. This gate runs before the filesystem is touched, so a remote
        // reference never costs a read.
        if let Some((scheme, _)) = value.split_once(':')
            && scheme.len() >= 2
            && is_scheme(scheme)
        {
            return Err(Error::Unsupported(format!(
                "unsupported locator scheme {scheme}: V1 supports file://, git:// and project:// sources"
            )));
        }
        Self::file(root, manifest, Path::new(value))
    }

    /// Canonical registration form for an evidence locator.
    ///
    /// Resolvable locators are stored as their canonical identity, so every writer
    /// records one shape. A remote or unavailable reference keeps the caller's
    /// spelling, matching the read path's existing treatment; whether registration
    /// should reject those is a separate question and is deliberately unchanged.
    pub fn normalize_for_registration(
        root: &Path,
        manifest: &Manifest,
        value: &str,
    ) -> Result<String> {
        match Self::parse(root, manifest, value) {
            Ok(locator) => locator.identity(),
            Err(Error::Unsupported(_)) | Err(Error::SourceUnavailable(_)) => Ok(value.to_owned()),
            Err(error) => Err(error),
        }
    }

    fn file(root: &Path, manifest: &Manifest, path: &Path) -> Result<Self> {
        if path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(Error::RuleViolation(
                "source path contains parent traversal".into(),
            ));
        }
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            root.join(path)
        }
        .canonicalize()
        .map_err(|e| Error::SourceUnavailable(e.to_string()))?;
        if !manifest
            .authorized_roots(root)?
            .iter()
            .any(|allowed| path.starts_with(allowed))
        {
            return Err(Error::RuleViolation(
                "source is outside authorized roots".into(),
            ));
        }
        Ok(Self::File(path))
    }
    pub fn read(&self, root: &Path, cap: u64) -> Result<SourceSnapshot> {
        match self {
            Self::File(path) => {
                let bytes = read_source_for_review(path, cap)?;
                let locator = Url::from_file_path(path)
                    .map_err(|_| Error::InvalidInput("invalid file locator".into()))?
                    .to_string();
                let content_review = crate::content_review::load_review(root, &locator, &bytes)?;
                let snapshot = SourceSnapshot {
                    locator,
                    fingerprint: fingerprint(&bytes),
                    bytes,
                    content_review,
                };
                snapshot.validate_content()?;
                Ok(snapshot)
            }
            Self::Project { path } => {
                let root = root.canonicalize()?;
                let resolved = root
                    .join(path)
                    .canonicalize()
                    .map_err(|e| Error::SourceUnavailable(e.to_string()))?;
                if !resolved.starts_with(&root) {
                    return Err(Error::RuleViolation(
                        "project source escapes the project root".into(),
                    ));
                }
                let bytes = read_source_for_review(&resolved, cap)?;
                let locator = self.identity()?;
                let content_review = crate::content_review::load_review(&root, &locator, &bytes)?;
                let snapshot = SourceSnapshot {
                    locator,
                    fingerprint: fingerprint(&bytes),
                    bytes,
                    content_review,
                };
                snapshot.validate_content()?;
                Ok(snapshot)
            }
            Self::Git { revision, path } => {
                let resolved = git(
                    root,
                    &[
                        "rev-parse",
                        "--verify",
                        "--end-of-options",
                        &format!("{revision}^{{commit}}"),
                    ],
                )?;
                let commit = std::str::from_utf8(&resolved)
                    .map_err(|e| Error::InvalidInput(e.to_string()))?
                    .trim()
                    .to_owned();
                if ![40, 64].contains(&commit.len())
                    || !commit.chars().all(|c| c.is_ascii_hexdigit())
                {
                    return Err(Error::SourceUnavailable(
                        "Git did not return an immutable commit".into(),
                    ));
                }
                let prefix = git(root, &["rev-parse", "--show-prefix"])?;
                let prefix = std::str::from_utf8(&prefix)
                    .map_err(|e| Error::InvalidInput(e.to_string()))?
                    .trim_end_matches('\n');
                let relative_path = path
                    .components()
                    .filter_map(|part| match part {
                        Component::Normal(value) => Some(value.to_str().ok_or_else(|| {
                            Error::InvalidInput("Git source path must be UTF-8".into())
                        })),
                        Component::CurDir => None,
                        _ => Some(Err(Error::RuleViolation(
                            "Git path escapes project root".into(),
                        ))),
                    })
                    .collect::<Result<Vec<_>>>()?
                    .join("/");
                let object = format!("{commit}:{prefix}{relative_path}");
                let size = git(root, &["cat-file", "-s", &object])?;
                let size = std::str::from_utf8(&size)
                    .map_err(|e| Error::InvalidInput(e.to_string()))?
                    .trim()
                    .parse::<u64>()
                    .map_err(|e| Error::SourceUnavailable(e.to_string()))?;
                if size > cap {
                    return Err(Error::InvalidInput(format!(
                        "Git source exceeds {cap} byte read cap"
                    )));
                }
                let bytes = git(root, &["cat-file", "blob", &object])?;
                if bytes.len() as u64 > cap {
                    return Err(Error::InvalidInput("Git blob exceeds read cap".into()));
                }
                let locator = format!("git://{commit}:{relative_path}");
                awr_core::ensure_public_source(&bytes, &locator)?;
                let mut digest = Sha256::new();
                digest.update(locator.as_bytes());
                digest.update([0]);
                digest.update(&bytes);
                Ok(SourceSnapshot {
                    locator,
                    fingerprint: format!("sha256:{:x}", digest.finalize()),
                    content_review: None,
                    bytes,
                })
            }
        }
    }
}
pub(crate) fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|e| Error::SourceUnavailable(format!("Git: {e}")))?;
    if !output.status.success() {
        return Err(Error::SourceUnavailable(
            String::from_utf8_lossy(&output.stderr).trim().into(),
        ));
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "[project]\nname = 'P'\n\n[[sources]]\ndomain = 'decisions'\nrole = 'supporting'\npath = 'decisions'\nadapter = 'markdown-directory-v1'\n";
    const GIT_REV: &str = "4bdeb5a384d6321b6b0dc22f69dfa71f283386bc";

    fn manifest() -> Manifest {
        Manifest::parse(MANIFEST).expect("test manifest")
    }

    fn missing_root() -> PathBuf {
        std::env::temp_dir().join("awr-locator-unit-missing-root-do-not-create")
    }

    fn snapshot(locator: &str, bytes: &[u8], digest: &str) -> SourceSnapshot {
        SourceSnapshot {
            locator: locator.into(),
            fingerprint: digest.into(),
            bytes: bytes.to_vec(),
            content_review: None,
        }
    }

    #[test]
    fn git_fingerprint_binds_resolved_locator_and_bytes() {
        let locator = format!("git://{GIT_REV}:docs/x.md");
        let bytes = b"hello\n";
        let mut hash = Sha256::new();
        hash.update(locator.as_bytes());
        hash.update([0]);
        hash.update(bytes);
        let expected = format!("sha256:{:x}", hash.finalize());
        assert!(snapshot(&locator, bytes, &expected).verify_fingerprint());
        // A different pinned commit is a different source even with equal bytes.
        let other = format!("git://{}:docs/x.md", "0".repeat(40));
        assert!(!snapshot(&other, bytes, &expected).verify_fingerprint());
        // The same locator with different bytes is not the reviewed snapshot.
        assert!(!snapshot(&locator, b"hello!\n", &expected).verify_fingerprint());
    }

    #[test]
    fn malformed_git_locators_fail_closed() {
        let bytes = b"hello\n";
        for locator in [
            format!("git://{GIT_REV}docs/x.md"),
            "git://abc:docs/x.md".to_string(),
            format!("git://{GIT_REV}:../x.md"),
            format!("git://{GIT_REV}:/abs.md"),
        ] {
            assert!(
                !snapshot(&locator, bytes, &fingerprint(bytes)).verify_fingerprint(),
                "{locator}"
            );
        }
    }

    #[test]
    fn file_fingerprint_is_sha256_of_bytes() {
        let bytes = b"hello\n";
        let locator = "file:///C:/proj/reports/a.md";
        let expected = "sha256:5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03";
        assert_eq!(fingerprint(bytes), expected);
        assert!(snapshot(locator, bytes, expected).verify_fingerprint());
        assert!(!snapshot(locator, b"hello", expected).verify_fingerprint());
    }

    #[test]
    fn unknown_scheme_is_not_a_fingerprint_authority() {
        for locator in ["https://example.com/x", "node:events"] {
            assert!(
                !snapshot(locator, b"hello\n", "sha256:00").verify_fingerprint(),
                "{locator}"
            );
        }
    }

    #[test]
    fn non_utf8_snapshot_text_is_invalid_input() {
        let bytes = [0xff, 0xfe, 0x00];
        let locator = "file:///C:/proj/reports/a.bin";
        let digest = fingerprint(&bytes);
        assert!(matches!(
            snapshot(locator, &bytes, &digest).text(),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn identity_shapes_are_canonical() {
        let absolute = std::env::temp_dir().join("awr-locator-identity/reports/a.md");
        let file_identity = Locator::File(absolute).identity().expect("file identity");
        assert!(file_identity.starts_with("file://"), "{file_identity}");
        assert!(file_identity.ends_with("/reports/a.md"), "{file_identity}");
        let git = Locator::Git {
            revision: GIT_REV.into(),
            path: PathBuf::from("docs/x.md"),
        };
        assert_eq!(
            git.identity().expect("git identity"),
            format!("git://{GIT_REV}:docs/x.md")
        );
        let escape = Locator::Git {
            revision: GIT_REV.into(),
            path: PathBuf::from("../x.md"),
        };
        assert!(matches!(escape.identity(), Err(Error::RuleViolation(_))));
    }

    #[test]
    fn unsupported_schemes_are_rejected_before_the_filesystem() {
        // The root does not exist: touching it would surface SourceUnavailable.
        for value in ["node:events", "s3:bucket/key", "https://example.com/x"] {
            assert!(
                matches!(
                    Locator::parse(&missing_root(), &manifest(), value),
                    Err(Error::Unsupported(_))
                ),
                "{value}"
            );
        }
    }

    #[test]
    fn windows_drive_letter_is_not_a_scheme() {
        // One-letter schemes are not schemes, so `C:\x` reaches the path branch.
        assert!(matches!(
            Locator::parse(&missing_root(), &manifest(), r"C:\x"),
            Err(Error::SourceUnavailable(_))
        ));
    }

    #[test]
    fn file_and_git_prefixes_without_slashes_are_unsupported() {
        // A bare path is the only path spelling. A scheme spelling that lost its
        // `//` is a protocol reference, never a relative file name.
        for value in ["file:reports/a.md", "git:reports/a.md"] {
            assert!(
                matches!(
                    Locator::parse(&missing_root(), &manifest(), value),
                    Err(Error::Unsupported(_))
                ),
                "{value}"
            );
        }
    }

    #[test]
    fn registration_normalization_keeps_unparsable_references_verbatim() {
        let manifest = manifest();
        for value in ["node:events", "https://example.com/x"] {
            assert_eq!(
                Locator::normalize_for_registration(&missing_root(), &manifest, value).unwrap(),
                value
            );
        }
        assert!(matches!(
            Locator::normalize_for_registration(&missing_root(), &manifest, "  "),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn manifest_locators_stay_scheme_qualified() {
        let spec = concat!(
            "[project]\nname = 'P'\n\n[[sources]]\ndomain = 'decisions'\n",
            "role = 'supporting'\nlocator = 'reports/a.md'\n",
            "adapter = 'markdown-directory-v1'\n"
        );
        let manifest = Manifest::parse(spec).expect("spec manifest");
        assert!(matches!(
            Locator::from_spec(&missing_root(), &manifest, &manifest.sources[0]),
            Err(Error::Unsupported(_))
        ));
    }
}
