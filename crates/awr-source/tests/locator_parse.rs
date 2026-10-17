//! Single-authority locator parsing: every accepted spelling resolves through
//! `awr_source::Locator::parse`, and path authority is checked there.

use awr_source::{Error, Locator, Manifest, SourceSnapshot, fingerprint};
use std::{env, fs, path::PathBuf};

const MANIFEST: &str = "[project]\nname = 'P'\n\n[[sources]]\ndomain = 'decisions'\nrole = 'supporting'\npath = 'decisions'\nadapter = 'markdown-directory-v1'\n";

fn manifest() -> Manifest {
    Manifest::parse(MANIFEST).expect("test manifest")
}

fn scratch(tag: &str) -> PathBuf {
    let root = env::temp_dir().join(format!("awr-locator-parse-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("reports")).expect("scratch root");
    fs::write(root.join("reports/a.md"), b"hello\n").expect("scratch report");
    root
}

#[test]
fn bare_path_normalizes_to_the_canonical_file_uri() {
    let root = scratch("bare");
    let parsed = Locator::parse(&root, &manifest(), "reports/a.md").expect("bare path parses");
    let identity = parsed.identity().expect("identity");
    assert!(identity.starts_with("file://"), "{identity}");
    let expected = root.join("reports/a.md").canonicalize().unwrap();
    match parsed {
        Locator::File(path) => assert_eq!(path, expected),
        other => panic!("bare path must parse as a file locator: {other:?}"),
    }
}

#[test]
fn registration_normalization_writes_one_canonical_shape() {
    let root = scratch("normalize");
    let manifest = manifest();
    let identity =
        Locator::normalize_for_registration(&root, &manifest, "reports/a.md").expect("normalizes");
    let canonical = root.join("reports/a.md").canonicalize().unwrap();
    assert_eq!(identity, Locator::File(canonical).identity().unwrap());
    // The canonical spelling is already stable, so re-registering it is a no-op.
    assert_eq!(
        Locator::normalize_for_registration(&root, &manifest, &identity).unwrap(),
        identity
    );
    // A remote reference keeps its spelling instead of failing registration.
    assert_eq!(
        Locator::normalize_for_registration(&root, &manifest, "node:events").unwrap(),
        "node:events"
    );
}

#[test]
fn canonical_file_uri_round_trips_byte_exactly() {
    let root = scratch("file");
    let canonical = root.join("reports/a.md").canonicalize().expect("canonical");
    let identity = Locator::File(canonical).identity().expect("identity");
    let parsed = Locator::parse(&root, &manifest(), &identity).expect("file:// parses");
    assert_eq!(parsed.identity().unwrap(), identity);
}

#[test]
fn pinned_git_locator_round_trips_byte_exactly() {
    let root = scratch("git");
    let identity = "git://4bdeb5a384d6321b6b0dc22f69dfa71f283386bc:docs/x.md";
    let parsed = Locator::parse(&root, &manifest(), identity).expect("git:// parses");
    assert_eq!(parsed.identity().unwrap(), identity);
}

#[test]
fn traversal_is_rejected_before_the_row_is_written() {
    let root = scratch("traversal");
    assert!(matches!(
        Locator::parse(&root, &manifest(), "../../etc/passwd"),
        Err(Error::RuleViolation(_))
    ));
}

#[test]
fn file_uri_outside_authorized_roots_is_rejected() {
    let root = scratch("outside");
    let outside = scratch("outside-target").join("reports/a.md");
    let identity = Locator::File(outside).identity().expect("identity");
    assert!(matches!(
        Locator::parse(&root, &manifest(), &identity),
        Err(Error::RuleViolation(_))
    ));
}

#[test]
fn remote_scheme_costs_no_filesystem_access() {
    // The root does not exist: if parsing touched the filesystem first, the error
    // would be SourceUnavailable instead of Unsupported.
    let missing = env::temp_dir().join("awr-locator-parse-missing-root");
    let _ = fs::remove_dir_all(&missing);
    for value in ["https://example.com/x", "node:events"] {
        assert!(matches!(
            Locator::parse(&missing, &manifest(), value),
            Err(Error::Unsupported(_))
        ));
    }
}

#[test]
fn file_uri_query_or_fragment_is_rejected() {
    let root = scratch("query");
    assert!(matches!(
        Locator::parse(&root, &manifest(), "file:///tmp/a.md?x=1"),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn fingerprints_stay_bound_to_bytes_and_locator() {
    let bytes = b"hello\n".to_vec();
    let file_locator = "file:///C:/proj/reports/a.md".to_string();
    let file_snapshot = SourceSnapshot {
        locator: file_locator,
        fingerprint: "sha256:5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03"
            .to_string(),
        bytes: bytes.clone(),
        content_review: None,
    };
    assert!(file_snapshot.verify_fingerprint());
    assert_eq!(file_snapshot.fingerprint, fingerprint(&file_snapshot.bytes));

    let git_locator = "git://4bdeb5a384d6321b6b0dc22f69dfa71f283386bc:docs/x.md".to_string();
    let git_snapshot = SourceSnapshot {
        locator: git_locator,
        fingerprint: "sha256:b2509b6a1aa26e5cda33b995af77e6429571813582a621f19ebc941315d668dd"
            .to_string(),
        bytes,
        content_review: None,
    };
    assert!(git_snapshot.verify_fingerprint());
}

#[test]
fn empty_and_parent_paths_are_rejected() {
    let root = scratch("empty");
    assert!(matches!(
        Locator::parse(&root, &manifest(), "   "),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        Locator::parse(
            &root,
            &manifest(),
            "git://4bdeb5a384d6321b6b0dc22f69dfa71f283386bc:../x"
        ),
        Err(Error::RuleViolation(_))
    ));
}
