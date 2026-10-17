use crate::{REGISTERED_CONTENT_READ_CAP, Runtime};
use awr_core::*;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

impl Runtime<'_> {
    /// Load current trusted policy before calling. The selected scope authorizes
    /// metadata before the filesystem is touched; existing path/hash checks still
    /// apply. This does not grant direct OS-file access or authorize execution.
    pub fn read_artifact_in_workstream(
        &self,
        access: &WorkstreamAccess,
        selection: &awr_store::WorkstreamReadSelection,
        id: Id,
        max_bytes: u64,
    ) -> Result<(Artifact, Vec<u8>)> {
        let view =
            self.store
                .read_workstream(self.project, access, selection, 256 * 1024 * 1024)?;
        let artifact = view.artifact(id)?;
        ensure_public_data(&artifact)?;
        let bytes = read_registered_file(
            self.store,
            self.project,
            &artifact.locator,
            Some(artifact.size),
            Some(&artifact.sha256),
            max_bytes,
        )?;
        let current =
            self.store
                .read_workstream(self.project, access, selection, 256 * 1024 * 1024)?;
        let checked = current.artifact(id)?;
        if checked.sha256 != artifact.sha256
            || checked.locator != artifact.locator
            || checked.revision != artifact.revision
        {
            return Err(Error::SourceConflict(
                "artifact changed during scoped read".into(),
            ));
        }
        Ok((artifact, bytes))
    }

    pub fn read_evidence_report_in_workstream(
        &self,
        access: &WorkstreamAccess,
        selection: &awr_store::WorkstreamReadSelection,
        key: &str,
        max_bytes: u64,
    ) -> Result<(EvidenceRecord, Vec<u8>)> {
        let view =
            self.store
                .read_workstream(self.project, access, selection, 256 * 1024 * 1024)?;
        let record = view.evidence(key)?;
        ensure_public_data(&record)?;
        let bytes = read_registered_file(
            self.store,
            self.project,
            &record.item.locator,
            None,
            record.item.sha256.as_deref(),
            max_bytes,
        )?;
        let current =
            self.store
                .read_workstream(self.project, access, selection, 256 * 1024 * 1024)?;
        let checked = current.evidence(&record.item.id.to_string())?;
        if checked.item.revision != record.item.revision
            || checked.item.locator != record.item.locator
            || checked.item.sha256 != record.item.sha256
        {
            return Err(Error::SourceConflict(
                "evidence changed during scoped read".into(),
            ));
        }
        Ok((record, bytes))
    }

    /// Explicit artifact body read. No content is returned until size and digest match its record.
    pub fn read_artifact(&self, id: Id, max_bytes: u64) -> Result<(Artifact, Vec<u8>)> {
        let artifact = self.store.artifact(self.project, id)?;
        ensure_public_data(&artifact)?;
        let bytes = read_registered_file(
            self.store,
            self.project,
            &artifact.locator,
            Some(artifact.size),
            Some(&artifact.sha256),
            max_bytes,
        )?;
        Ok((artifact, bytes))
    }

    /// Read a registered local report on demand. Absence of a recorded digest is not verification.
    pub fn read_evidence_report(
        &self,
        key: &str,
        max_bytes: u64,
    ) -> Result<(EvidenceRecord, Vec<u8>)> {
        let record = self.store.evidence(self.project, key)?;
        ensure_public_data(&record)?;
        let bytes = read_registered_file(
            self.store,
            self.project,
            &record.item.locator,
            None,
            record.item.sha256.as_deref(),
            max_bytes,
        )?;
        Ok((record, bytes))
    }
}

/// One resolved form for every accepted evidence-locator spelling.
///
/// The locator authority decides which file a string names, so currency decisions
/// and the recorded-byte check compare the canonical path and the bytes, never the
/// recorded text: `file://<absolute>`, the bare absolute path and a project-relative
/// path with redundant `.` segments all resolve to the same file. Grouping elsewhere
/// still keys on the recorded string; this helper never rewrites a row.
///
/// `None` means the spelling does not name a registered local file (a `git://`
/// reference or any other scheme, remote or not); each caller keeps its own refusal.
pub(crate) fn resolve_registered_locator(root: &Path, locator: &str) -> Result<Option<PathBuf>> {
    if locator.starts_with("file://") {
        let manifest = awr_source::Manifest::load(root)?;
        return match awr_source::Locator::parse(root, &manifest, locator)? {
            awr_source::Locator::File(path) => Ok(Some(path)),
            awr_source::Locator::Git { .. } => Ok(None),
        };
    }
    if locator.contains("://") {
        return Ok(None);
    }
    Ok(Some(
        if Path::new(locator).is_absolute() {
            Path::new(locator).to_path_buf()
        } else {
            root.join(locator)
        }
        .canonicalize()?,
    ))
}

pub(crate) fn read_registered_file(
    store: &awr_store::Store,
    project: Id,
    locator: &str,
    expected_size: Option<u64>,
    expected_sha: Option<&str>,
    max_bytes: u64,
) -> Result<Vec<u8>> {
    if max_bytes == 0 || max_bytes > REGISTERED_CONTENT_READ_CAP {
        return Err(Error::InvalidInput(
            "read limit must be 1..16777216 bytes".into(),
        ));
    }
    if expected_size.is_some_and(|size| size > max_bytes) {
        return Err(Error::InvalidInput(format!(
            "artifact exceeds {max_bytes} byte read limit"
        )));
    }
    let project = store.project(project)?;
    let root = project.root.canonicalize()?;
    if root != project.root {
        return Err(Error::SourceConflict(
            "project root moved since registration".into(),
        ));
    }
    // One resolved form for every accepted spelling; the byte check below still
    // binds the bytes, so an alias cannot stand in for different content.
    let path = resolve_registered_locator(&root, locator)?.ok_or_else(|| {
        Error::Unsupported("body reads currently support registered local paths; use the report locator for remote content".into())
    })?;
    let allowed = path.starts_with(&root)
        || (root.join(".awr/project.toml").exists()
            && awr_source::Manifest::load(&root)?
                .project
                .authorized_roots
                .iter()
                .any(|p| {
                    root.join(p)
                        .canonicalize()
                        .is_ok_and(|r| path.starts_with(r))
                }));
    if !allowed {
        return Err(Error::RuleViolation(
            "registered file is outside project and authorized roots".into(),
        ));
    }
    if !path.metadata()?.is_file() {
        return Err(Error::InvalidInput(
            "registered content is not a regular file".into(),
        ));
    }
    let file = awr_source::open_file_exact(&path)?;
    let before = file.metadata()?;
    if !before.is_file() {
        return Err(Error::InvalidInput(
            "registered content is not a regular file".into(),
        ));
    }
    if before.len() > max_bytes {
        return Err(Error::InvalidInput(format!(
            "content has {} bytes; limit is {max_bytes}",
            before.len()
        )));
    }
    if expected_size.is_some_and(|size| size != before.len()) {
        return Err(Error::SourceConflict(
            "artifact size differs from its registered metadata".into(),
        ));
    }
    let mut bytes = Vec::with_capacity(before.len() as usize);
    (&file).take(max_bytes + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(Error::InvalidInput("content grew beyond read limit".into()));
    }
    let after = file.metadata()?;
    if bytes.len() as u64 != before.len()
        || after.len() != before.len()
        || after.modified()? != before.modified()?
    {
        return Err(Error::SourceConflict(
            "registered content changed during reading".into(),
        ));
    }
    if let Some(expected) = expected_sha {
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(Error::SourceConflict(format!(
                "registered content bytes no longer match the SHA256 bound at registration: expected {expected}, actual {actual}; the file must stay byte-identical, so check for added or removed trailing newlines, whitespace or line-ending changes since the evidence was registered"
            )));
        }
    }
    ensure_public_bytes(&bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, fs};

    const MANIFEST: &str = "[project]\nname = 'P'\n\n[[sources]]\ndomain = 'decisions'\nrole = 'supporting'\npath = 'decisions'\nadapter = 'markdown-directory-v1'\n";

    fn scratch(tag: &str) -> PathBuf {
        let root = env::temp_dir().join(format!("awr-spelling-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".awr")).expect("scratch runtime dir");
        fs::write(root.join(".awr/project.toml"), MANIFEST).expect("scratch manifest");
        fs::create_dir_all(root.join("reports")).expect("scratch reports");
        fs::write(root.join("reports/a.md"), b"hello\n").expect("scratch report");
        root.canonicalize().expect("canonical scratch root")
    }

    #[test]
    fn every_spelling_of_one_file_resolves_to_the_same_path() {
        let root = scratch("same");
        let canonical = root.join("reports/a.md").canonicalize().unwrap();
        let identity = awr_source::Locator::File(canonical.clone())
            .identity()
            .expect("file identity");
        let absolute = root
            .join("reports/a.md")
            .to_str()
            .expect("utf8 path")
            .to_string();
        let resolved: Vec<PathBuf> = [identity, absolute, "reports/./a.md".into()]
            .iter()
            .map(|locator| {
                resolve_registered_locator(&root, locator)
                    .expect("spelling resolves")
                    .expect("local file")
            })
            .collect();
        assert!(
            resolved.iter().all(|path| path == &canonical),
            "{resolved:?}"
        );
    }

    #[test]
    fn different_files_are_not_equivalent_even_with_equal_bytes() {
        let left = scratch("left");
        let right = scratch("right");
        // Both roots hold `reports/a.md` with identical bytes; a resolved path is
        // still per-project, so equal content can never make two files equivalent.
        let left_path = resolve_registered_locator(&left, "reports/a.md")
            .unwrap()
            .expect("local file");
        let right_path = resolve_registered_locator(&right, "reports/a.md")
            .unwrap()
            .expect("local file");
        assert_ne!(left_path, right_path);
        assert_eq!(left_path, left.join("reports/a.md").canonicalize().unwrap());
    }

    #[test]
    fn remote_and_git_spellings_never_resolve_to_a_local_report() {
        let root = scratch("remote");
        for locator in [
            "git://4bdeb5a384d6321b6b0dc22f69dfa71f283386bc:docs/x.md",
            "https://example.com/x",
        ] {
            assert!(
                resolve_registered_locator(&root, locator)
                    .expect("a remote reference is not an error")
                    .is_none(),
                "{locator}"
            );
        }
    }

    #[test]
    fn a_scheme_like_bare_token_keeps_the_legacy_path_branch() {
        // `node:events` never carried a `file://` spelling, so it keeps the original
        // rule: a relative path that fails as missing, not a remote reference.
        let root = scratch("scheme");
        assert!(matches!(
            resolve_registered_locator(&root, "node:events"),
            Err(Error::Io(_))
        ));
    }
}
