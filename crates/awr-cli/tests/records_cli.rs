use awr_core::Id;
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("awr-records-cli-{}", Id::new()));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("decisions")).unwrap();
        fs::write(
            root.join("work.yaml"),
            "work_items:\n- id: W\n  title: Implement records\n  status: ready\n",
        )
        .unwrap();
        fs::write(root.join("decisions/ADR-1.md"),"# ADR-1: Keep reports external\n\nStatus: accepted\n\n## Decision\n\nKeep report metadata in SQLite.\n\n## Rationale\n\nPRIVATE_RATIONALE_SENTINEL\n").unwrap();
        fs::write(root.join("sources.toml"),"[project]\nname='Records'\nexternal_key='records'\nauthority_mode='source_first'\nauthorized_roots=[]\n\n[[sources]]\ndomain='ledger'\nrole='primary'\npath='work.yaml'\nadapter='yaml-ledger-v1'\n\n[[sources]]\ndomain='decisions'\nrole='supporting'\npath='decisions'\nadapter='markdown-directory-v1'\n").unwrap();
        let f = Self(root);
        f.ok(&["init", "--manifest", "sources.toml", "--accept"]);
        f.ok(&["status"]);
        f
    }
    /// A work ledger authored as Markdown: its work items carry no per-item
    /// identity+acceptance section, only the whole-file source revision.
    fn markdown_ledger() -> Self {
        let root = std::env::temp_dir().join(format!("awr-records-md-{}", Id::new()));
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join("work.md"),
            "# Reading ledger\n\n- [ ] Read notes <!-- awr:id=\"W\" --> <!-- awr:acceptance=[\"Useful note\"] -->\n",
        )
        .unwrap();
        fs::write(
            root.join("mapping.toml"),
            "[project]\nname='Markdown work'\n\n[[sources]]\ndomain='ledger'\nrole='primary'\npath='work.md'\nadapter='markdown-ledger-v1'\n",
        )
        .unwrap();
        let f = Self(root);
        f.ok(&["init", "--manifest", "mapping.toml", "--accept"]);
        f.ok(&["status"]);
        f
    }
    fn write(&self, name: &str, text: &str) {
        fs::write(self.0.join(name), text).unwrap();
    }
    fn run(&self, args: &[&str], json: bool) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_awr"));
        cmd.arg("--project").arg(&self.0);
        if json {
            cmd.arg("--json");
        }
        cmd.args(args).output().unwrap()
    }
    fn ok(&self, args: &[&str]) -> Value {
        let r = self.run(args, true);
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        serde_json::from_slice(&r.stdout).unwrap()
    }
    fn error(&self, args: &[&str], code: &str) {
        let r = self.run(args, true);
        assert!(!r.status.success());
        assert!(r.stdout.is_empty());
        assert_eq!(
            serde_json::from_slice::<Value>(&r.stderr).unwrap()["code"],
            code
        );
    }
    fn revision(&self) -> String {
        self.ok(&["session", "list"])["project_revision"].to_string()
    }
    fn import(&self, body: &[u8]) -> Value {
        fs::write(self.0.join("report.bin"), body).unwrap();
        let session = self.ok(&[
            "session",
            "start",
            "--work",
            "W",
            "--agent",
            "producer",
            "--provider",
            "fixture",
            "--model",
            "test",
            "--expected-revision",
            &self.revision(),
        ]);
        self.ok(&[
            "artifact",
            "add",
            "report.bin",
            "--type",
            "report",
            "--mime",
            "application/octet-stream",
            "--source-event",
            session["event"]["id"].as_str().unwrap(),
            "--expected-revision",
            &session["project_revision"].to_string(),
        ])
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn explicit_report_reads_are_bounded_hash_checked_and_do_not_promote_work() {
    let f = Fixture::new();
    let body = "REPORT_BODY_SENTINEL\n".repeat(8000);
    let imported = f.import(body.as_bytes());
    let artifact = &imported["artifact"];
    let aid = artifact["id"].as_str().unwrap();
    let metadata = f.ok(&["artifact", "show", aid]);
    assert!(!metadata.to_string().contains("REPORT_BODY_SENTINEL"));
    f.error(&["artifact", "cat", aid], "InvalidInput");
    let cat = f.ok(&["artifact", "cat", aid, "--max-bytes", "200000"]);
    assert_eq!(cat["content"], body);
    assert_eq!(cat["content_hash_verified"], true);
    let draft = json!({"external_key":"CHECK-1","work_item_key":"W","evidence_type":"report","level":"locally_verified","summary":"Direct behavior check","locator":artifact["locator"],"sha256":artifact["sha256"],"source_sha":"a".repeat(40),"command":"cargo test --test records_cli","scope":["W"],"branch_id":null,"verified_at":1});
    fs::write(
        f.0.join("evidence.json"),
        serde_json::to_vec(&draft).unwrap(),
    )
    .unwrap();
    f.error(
        &[
            "evidence",
            "add",
            "--input",
            "evidence.json",
            "--expected-revision",
            "0",
        ],
        "RevisionConflict",
    );
    let added = f.ok(&[
        "evidence",
        "add",
        "--input",
        "evidence.json",
        "--expected-revision",
        &f.revision(),
    ]);
    let eid = added["evidence"]["id"].as_str().unwrap();
    assert_eq!(added["validation_basis"], "caller_supplied_bindings");
    let brief = f.ok(&["evidence", "show", "CHECK-1"]);
    assert_eq!(brief["evidence"]["id"], eid);
    assert_eq!(brief["currency"], "unknown");
    assert!(brief.get("content").is_none());
    assert!(!brief.to_string().contains("REPORT_BODY_SENTINEL"));
    let current = f.ok(&[
        "evidence",
        "show",
        eid,
        "--source-sha",
        &"a".repeat(40),
        "--content",
        "--max-bytes",
        "200000",
    ]);
    assert_eq!(current["currency"], "current");
    assert_eq!(current["content"], body);
    assert_eq!(
        f.ok(&["evidence", "show", eid, "--source-sha", &"b".repeat(40)])["currency"],
        "historical"
    );
    assert_eq!(f.ok(&["work", "show", "W"])["work"]["status"], "ready");
    let revision = f.revision();
    fs::write(
        f.0.join(artifact["locator"].as_str().unwrap()),
        "X".repeat(body.len()),
    )
    .unwrap();
    f.error(
        &["artifact", "cat", aid, "--max-bytes", "200000"],
        "SourceConflict",
    );
    f.error(
        &[
            "evidence",
            "show",
            eid,
            "--content",
            "--max-bytes",
            "200000",
        ],
        "SourceConflict",
    );
    assert_eq!(f.revision(), revision);
    assert!(f.ok(&["artifact", "show", aid]).get("content").is_none());
    let diagnosed = f.run(&["doctor"], true);
    assert!(!diagnosed.status.success());
    let diagnosis: Value = serde_json::from_slice(&diagnosed.stdout).unwrap();
    assert_eq!(diagnosis["database_ok"], true);
    assert!(
        diagnosis["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["code"] == "artifact_digest_mismatch")
    );
    assert_eq!(f.revision(), revision);
}

#[test]
fn decision_rationale_requires_explicit_full_read_and_accepts_internal_id() {
    let f = Fixture::new();
    let hits = f.ok(&["search", "--type", "decision"]);
    let key = hits["hits"][0]["external_key"].as_str().unwrap();
    let brief = f.ok(&["decision", "show", key]);
    assert!(!brief.to_string().contains("PRIVATE_RATIONALE_SENTINEL"));
    assert!(brief["source_ref"].is_object());
    let id = brief["decision"]["id"].as_str().unwrap();
    let full = f.ok(&["decision", "show", id, "--full"]);
    assert!(
        full["decision"]["rationale"]
            .as_str()
            .unwrap()
            .contains("PRIVATE_RATIONALE_SENTINEL")
    );
    f.error(
        &["decision", "show", id, "--full", "--max-bytes", "1"],
        "InvalidInput",
    );
    f.error(&["decision", "show", "MISSING"], "NotFound");
}

#[test]
fn binary_artifact_raw_output_is_exact_and_verified_evidence_needs_bindings() {
    let f = Fixture::new();
    let body = [0u8, 255, 128, 13, 10];
    let imported = f.import(&body);
    let id = imported["artifact"]["id"].as_str().unwrap();
    f.error(&["artifact", "cat", id], "Unsupported");
    let raw = f.run(&["artifact", "cat", id], false);
    assert!(raw.status.success());
    assert_eq!(raw.stdout, body);
    let draft = json!({"external_key":"INCOMPLETE","work_item_key":"W","evidence_type":"report","level":"locally_verified","summary":"Missing verification binding","locator":"report.bin","sha256":null,"source_sha":null,"command":null,"scope":["W"],"branch_id":null,"verified_at":null});
    fs::write(
        f.0.join("incomplete.json"),
        serde_json::to_vec(&draft).unwrap(),
    )
    .unwrap();
    let revision = f.revision();
    f.error(
        &[
            "evidence",
            "add",
            "--input",
            "incomplete.json",
            "--expected-revision",
            &revision,
        ],
        "EvidenceMissing",
    );
    assert_eq!(f.revision(), revision);
    f.error(&["evidence", "show", "INCOMPLETE"], "NotFound");
}

#[test]
fn a_changed_work_section_stops_evidence_from_reporting_current() {
    let f = Fixture::new();
    let before = f.ok(&["work", "show", "W"]);
    let source_sha = before["source_ref"]["source_fingerprint"]
        .as_str()
        .unwrap()
        .trim_start_matches("sha256:")
        .to_string();
    let section_before = before["source_ref"]["section_fingerprint"].clone();
    // A yaml-ledger work item carries a per-item identity+acceptance section.
    assert!(section_before.is_string());
    let draft = json!({"external_key":"SECTION-CHECK","work_item_key":"W","evidence_type":"report","level":"locally_verified","summary":"Bound to one work section","locator":"section-report.txt","sha256":"a".repeat(64),"source_sha":source_sha,"command":"cargo test --test records_cli","scope":["W"],"branch_id":null,"verified_at":1});
    fs::write(f.0.join("section-report.txt"), "SECTION_REPORT_SENTINEL\n").unwrap();
    fs::write(
        f.0.join("section.json"),
        serde_json::to_vec(&draft).unwrap(),
    )
    .unwrap();
    f.ok(&[
        "evidence",
        "add",
        "--input",
        "section.json",
        "--expected-revision",
        &f.revision(),
    ]);
    assert_eq!(
        f.ok(&[
            "evidence",
            "show",
            "SECTION-CHECK",
            "--source-sha",
            &source_sha
        ])["currency"],
        "current"
    );

    // Edit the work item's identity+acceptance section while the ledger source SHA
    // the evidence was recorded against stays the requested one.
    fs::write(
        f.0.join("work.yaml"),
        "work_items:\n- id: W\n  title: Implement records and more\n  status: ready\n",
    )
    .unwrap();
    f.ok(&["source", "reindex"]);
    let after = f.ok(&["work", "show", "W"]);
    assert_ne!(after["source_ref"]["section_fingerprint"], section_before);
    let shown = f.ok(&[
        "evidence",
        "show",
        "SECTION-CHECK",
        "--source-sha",
        &source_sha,
    ]);
    assert_eq!(shown["currency"], "historical");
    assert!(
        shown["reasons"]
            .to_string()
            .contains("different work section")
    );
}

/// A Markdown work ledger has no per-item section binding, so currency must stay
/// on the whole-file source revision: no section is invented, and a later source
/// revision still makes the old record historical.
#[test]
fn a_work_item_without_a_section_binding_keeps_source_revision_currency() {
    let f = Fixture::markdown_ledger();
    let before = f.ok(&["work", "show", "W"]);
    let source_sha = before["source_ref"]["source_fingerprint"]
        .as_str()
        .unwrap()
        .trim_start_matches("sha256:")
        .to_string();
    assert!(before["source_ref"]["section_fingerprint"].is_null());
    let draft = json!({"external_key":"MD-CHECK","work_item_key":"W","evidence_type":"report","level":"locally_verified","summary":"Bound to a Markdown work item","locator":"md-report.txt","sha256":"a".repeat(64),"source_sha":source_sha,"command":"cargo test --test records_cli","scope":["W"],"branch_id":null,"verified_at":1});
    f.write("md-report.txt", "MD_REPORT_SENTINEL\n");
    f.write("md.json", &serde_json::to_string(&draft).unwrap());
    f.ok(&[
        "evidence",
        "add",
        "--input",
        "md.json",
        "--expected-revision",
        &f.revision(),
    ]);
    assert_eq!(
        f.ok(&["evidence", "show", "MD-CHECK", "--source-sha", &source_sha])["currency"],
        "current"
    );

    f.write(
        "work.md",
        "# Reading ledger\n\n- [ ] Read notes deeply <!-- awr:id=\"W\" --> <!-- awr:acceptance=[\"Useful note\"] -->\n",
    );
    f.ok(&["source", "reindex"]);
    let after = f.ok(&["work", "show", "W"]);
    let next_sha = after["source_ref"]["source_fingerprint"]
        .as_str()
        .unwrap()
        .trim_start_matches("sha256:")
        .to_string();
    assert_ne!(next_sha, source_sha);
    // Still no per-item section: the recorded revision is still the requested one.
    assert!(after["source_ref"]["section_fingerprint"].is_null());
    assert_eq!(
        f.ok(&["evidence", "show", "MD-CHECK", "--source-sha", &source_sha])["currency"],
        "current"
    );
    assert_eq!(
        f.ok(&["evidence", "show", "MD-CHECK", "--source-sha", &next_sha])["currency"],
        "historical"
    );
}
