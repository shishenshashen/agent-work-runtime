//! A locator spelling never decides a currency answer or merges two records.
//!
//! Currency is relative to explicitly requested bindings, so two spellings of one
//! file must answer identically; presentation grouping keeps keying on the recorded
//! string, so those same two spellings stay two groups.

use awr_core::{Evidence, EvidenceCurrency, EvidenceLevel, EvidenceRecord, Id, evidence_groups};

fn record(locator: &str) -> EvidenceRecord {
    EvidenceRecord {
        item: Evidence {
            id: Id::new(),
            project_id: Id::new(),
            work_item_id: None,
            external_key: "REPORT-1".into(),
            evidence_type: "verification_report".into(),
            level: EvidenceLevel::LocallyVerified,
            summary: "Reviewed report".into(),
            locator: locator.into(),
            sha256: Some("0".repeat(64)),
            source_sha: Some("0".repeat(40)),
            section_fingerprint: Some("section-fp".into()),
            command: Some("true".into()),
            scope: vec!["W".into()],
            source_ref: None,
            branch_id: None,
            revision: 0,
            verified_at: Some(1),
        },
        source: None,
        project_revision: 0,
    }
}

#[test]
fn locator_spelling_never_changes_the_currency_answer() {
    let source = "0".repeat(40);
    let answer = |locator: &str| {
        let assessment = record(locator).assess(Some(&source), Some("section-fp"), None);
        (assessment.currency, assessment.reasons)
    };
    let canonical = answer("file:///C:/proj/reports/a.md");
    assert_eq!(canonical.0, EvidenceCurrency::Current);
    // `file://<absolute>`, the bare absolute path and a relative spelling with a
    // redundant `.` all name the same report and therefore answer identically.
    assert_eq!(answer("C:\\proj\\reports\\a.md"), canonical);
    assert_eq!(answer("reports/./a.md"), canonical);
}

#[test]
fn grouping_still_keys_on_the_recorded_string() {
    let source = "0".repeat(40);
    let assessments: Vec<_> = ["file:///C:/proj/reports/a.md", "reports/a.md"]
        .into_iter()
        .map(|locator| record(locator).assess(Some(&source), Some("section-fp"), None))
        .collect();
    let groups = evidence_groups(&assessments);
    assert_eq!(groups.len(), 2, "{groups:?}");
    for group in &groups {
        assert_eq!(
            group["records"].as_array().map(Vec::len),
            Some(1),
            "{group}"
        );
    }
}
