//! Spec tests for cross-device session handoff — design FEAT-XDH-001 v1.0.
//!
//! See `reports/feature-cross-device-handoff-design-v1.0.md` §3.2–§3.4 (ticket, clock
//! normalisation, redemption) and §4 (A1–A8).
//!
//! TODO(design-approval): replace the throwaway `reference` module below with
//! `awr_core::device_handoff` once the production module lands. Until then this file pins the
//! *specification* so the redemption rule can be reviewed and run before any storage change.
//!
//! TODO(verify): **never executed** on the authoring host — `cargo`, `rustc` and `link.exe` do not
//! resolve there at all, so this file has never been compiled and is not claimed to be
//! `cargo fmt` clean. A passing result may only be quoted with a CI run id.

use sha2::{Digest, Sha256};

/// Throwaway reference implementation of the normative redemption rule. Not production code.
mod reference {
    use serde_json::{json, Value};
    use std::collections::BTreeMap;

    /// A portable handoff ticket (design §3.2). Search keys are `work_key` / `branch_name`;
    /// every ULID field is audit-only, because ULID spaces of two devices do not intersect.
    #[derive(Debug, Clone, PartialEq)]
    pub struct HandoffTicket {
        pub ticket_id: String,
        pub work_key: String,
        pub branch_name: String,
        pub from_node_id: String,
        pub to_node_id: String,
        pub sender_session_id: String,
        pub checkpoint_id: String,
        pub checkpoint_digest: String,
        /// Sender's own wall clock. Audit-only: the rule must never read it (§3.3).
        pub sender_clock_ms: i64,
        pub issued_at_ms: i64,
        /// `deadline − sender_clock` as the sender believed it.
        pub remaining_ms: i64,
        pub closed_claim_ids: Vec<String>,
    }

    /// Receiver-side facts. Nothing here is trusted from the sender's clock but `remaining_ms`.
    #[derive(Debug, Clone)]
    pub struct RedeemInput {
        pub receiver_node_id: String,
        pub receiver_at_ms: i64,
        pub rtt_ms: i64,
        pub target_resolved: bool,
        pub local_checkpoint_digest: String,
        pub redeemed_ticket_ids: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum HandoffError {
        NodeMismatch { expected: String, got: String },
        TargetUnresolved { work_key: String, branch_name: String },
        CheckpointDrift { expected: String, got: String },
        TicketAlreadyRedeemed { ticket_id: String },
        ClaimExpired { remaining_ms: i64 },
    }

    /// Redemption validation, in the fixed order of design §3.4. The order is part of the
    /// contract: a ticket that is wrong in two ways must fail the same way every time, so the
    /// refusal is reproducible rather than a product of check scheduling.
    pub fn validate_ticket(
        t: &HandoffTicket,
        i: &RedeemInput,
    ) -> Result<(), HandoffError> {
        if t.to_node_id != i.receiver_node_id {
            return Err(HandoffError::NodeMismatch {
                expected: t.to_node_id.clone(),
                got: i.receiver_node_id.clone(),
            });
        }
        if !i.target_resolved {
            return Err(HandoffError::TargetUnresolved {
                work_key: t.work_key.clone(),
                branch_name: t.branch_name.clone(),
            });
        }
        if i.local_checkpoint_digest != t.checkpoint_digest {
            return Err(HandoffError::CheckpointDrift {
                expected: t.checkpoint_digest.clone(),
                got: i.local_checkpoint_digest.clone(),
            });
        }
        if i.redeemed_ticket_ids.iter().any(|id| id == &t.ticket_id) {
            return Err(HandoffError::TicketAlreadyRedeemed {
                ticket_id: t.ticket_id.clone(),
            });
        }
        Ok(())
    }

    /// Clock normalisation (design §3.3): the receiver rebuilds the deadline on its own clock and
    /// may only **shorten** it. Mirrors the upstream precedent in
    /// `crates/awr-team-pg/src/scoped_runner.rs:119` — subtract the entire round trip.
    pub fn local_deadline_ms(
        t: &HandoffTicket,
        receiver_at_ms: i64,
        rtt_ms: i64,
    ) -> Result<i64, HandoffError> {
        if t.remaining_ms <= 0 {
            return Err(HandoffError::ClaimExpired {
                remaining_ms: t.remaining_ms,
            });
        }
        let budget = t.remaining_ms.saturating_sub(rtt_ms);
        Ok(receiver_at_ms.saturating_add(budget))
    }

    /// Canonical, byte-stable ticket JSON: sorted keys, integers only, `closed_claim_ids` sorted.
    pub fn canonical_ticket_json(t: &HandoffTicket) -> Value {
        let mut closed = t.closed_claim_ids.clone();
        closed.sort();
        let mut m = BTreeMap::new();
        m.insert("branch_name", json!(t.branch_name));
        m.insert("checkpoint_digest", json!(t.checkpoint_digest));
        m.insert("checkpoint_id", json!(t.checkpoint_id));
        m.insert("closed_claim_ids", json!(closed));
        m.insert("from_node_id", json!(t.from_node_id));
        m.insert("issued_at_ms", json!(t.issued_at_ms));
        m.insert("remaining_ms", json!(t.remaining_ms));
        m.insert("sender_clock_ms", json!(t.sender_clock_ms));
        m.insert("sender_session_id", json!(t.sender_session_id));
        m.insert("ticket_id", json!(t.ticket_id));
        m.insert("to_node_id", json!(t.to_node_id));
        m.insert("work_key", json!(t.work_key));
        Value::Object(m.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
}

use reference::{
    canonical_ticket_json, local_deadline_ms, validate_ticket, HandoffError, HandoffTicket,
    RedeemInput,
};
use serde_json::Value;

const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn ticket(to_node: &str, remaining_ms: i64, sender_clock_ms: i64, digest: &str) -> HandoffTicket {
    HandoffTicket {
        ticket_id: "01J0TICKET0000000000000001".to_string(),
        work_key: "FEAT-XDH-001".to_string(),
        branch_name: "feat/cross-device-session-handoff".to_string(),
        from_node_id: "sha256:aaaa".to_string(),
        to_node_id: to_node.to_string(),
        sender_session_id: "01J0SENDER0000000000000001".to_string(),
        checkpoint_id: "01J0CHECKPOINT00000000001".to_string(),
        checkpoint_digest: digest.to_string(),
        sender_clock_ms,
        issued_at_ms: sender_clock_ms,
        remaining_ms,
        closed_claim_ids: vec!["01J0CLAIM-B".to_string(), "01J0CLAIM-A".to_string()],
    }
}

fn input(node: &str, at: i64, rtt: i64, resolved: bool, digest: &str) -> RedeemInput {
    RedeemInput {
        receiver_node_id: node.to_string(),
        receiver_at_ms: at,
        rtt_ms: rtt,
        target_resolved: resolved,
        local_checkpoint_digest: digest.to_string(),
        redeemed_ticket_ids: Vec::new(),
    }
}

fn sha256_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A1 — a ticket addressed to another device is refused.
#[test]
fn a1_ticket_for_another_node_is_refused() {
    let t = ticket("sha256:bbbb", 60_000, 1_000, DIGEST);
    let i = input("sha256:cccc", 2_000, 0, true, DIGEST);
    match validate_ticket(&t, &i) {
        Err(HandoffError::NodeMismatch { expected, got }) => {
            assert_eq!(expected, "sha256:bbbb");
            assert_eq!(got, "sha256:cccc");
        }
        other => panic!("expected NodeMismatch, got {other:?}"),
    }
}

/// A2 — the receiver's deadline is a fixed function of its own clock and `remaining_ms` only.
#[test]
fn a2_receiver_deadline_ignores_the_sender_clock_and_never_extends() {
    let receiver_at = 10_000_000i64;
    let rtt = 250i64;
    let remaining = 60_000i64;

    let mut observed = Vec::new();
    for sender_clock in [
        receiver_at - 10_000_000,
        receiver_at,
        receiver_at + 10_000_000,
    ] {
        let t = ticket("node-b", remaining, sender_clock, DIGEST);
        let deadline = local_deadline_ms(&t, receiver_at, rtt).expect("positive budget");
        // Upper bound: never beyond what the sender believed, minus the whole round trip.
        assert!(
            deadline <= receiver_at + remaining - rtt,
            "deadline extends beyond the sender's belief"
        );
        observed.push(deadline);
    }
    assert_eq!(
        observed[0], observed[1],
        "deadline moved with the sender clock (−10^7 vs 0)"
    );
    assert_eq!(
        observed[1], observed[2],
        "deadline moved with the sender clock (0 vs +10^7)"
    );

    // Monotone in the round trip: a slower transfer can only shorten the budget.
    let slower = local_deadline_ms(&ticket("node-b", remaining, 0, DIGEST), receiver_at, rtt * 4)
        .expect("positive budget");
    assert!(slower < observed[0], "a larger rtt must strictly shorten the deadline");
}

/// A3 — redeeming the same ticket twice must be refused (exactly-once at the rule level).
#[test]
fn a3_second_redemption_of_the_same_ticket_is_refused() {
    let t = ticket("node-b", 60_000, 1_000, DIGEST);
    let mut i = input("node-b", 2_000, 10, true, DIGEST);
    assert!(validate_ticket(&t, &i).is_ok(), "first redemption must validate");
    i.redeemed_ticket_ids.push(t.ticket_id.clone());
    match validate_ticket(&t, &i) {
        Err(HandoffError::TicketAlreadyRedeemed { ticket_id }) => {
            assert_eq!(ticket_id, t.ticket_id)
        }
        other => panic!("expected TicketAlreadyRedeemed, got {other:?}"),
    }
}

/// A4 — a ticket with no remaining budget is refused rather than turned into a fresh lease.
#[test]
fn a4_non_positive_remaining_budget_is_refused() {
    for remaining in [0i64, -1, -60_000] {
        let t = ticket("node-b", remaining, 1_000, DIGEST);
        match local_deadline_ms(&t, 2_000, 0) {
            Err(HandoffError::ClaimExpired { remaining_ms }) => assert_eq!(remaining_ms, remaining),
            other => panic!("expected ClaimExpired for remaining={remaining}, got {other:?}"),
        }
    }
}

/// A5 — a checkpoint whose content no longer matches the ticket digest is a drift, not an inherit.
#[test]
fn a5_checkpoint_drift_is_refused() {
    let t = ticket("node-b", 60_000, 1_000, DIGEST);
    let i = input("node-b", 2_000, 0, true, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    match validate_ticket(&t, &i) {
        Err(HandoffError::CheckpointDrift { expected, got }) => {
            assert_eq!(expected, DIGEST);
            assert_ne!(expected, got);
        }
        other => panic!("expected CheckpointDrift, got {other:?}"),
    }
}

/// A6 — the ticket record is reproducible and carries no floating point.
#[test]
fn a6_ticket_record_is_reproducible_and_hashable() {
    let t = ticket("node-b", 60_000, 1_000, DIGEST);
    let a = canonical_ticket_json(&t).to_string();
    let b = canonical_ticket_json(&t.clone()).to_string();
    assert_eq!(a, b, "ticket record is not reproducible for an identical input");
    assert!(!a.contains('.'), "ticket record must contain no floating point: {a}");
    let hash = sha256_hex(&a);
    assert_eq!(hash.len(), 64);
    assert!(hash.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));

    // Claim-id ordering must not leak into the record.
    let mut shuffled = t.clone();
    shuffled.closed_claim_ids.reverse();
    assert_eq!(
        canonical_ticket_json(&shuffled).to_string(),
        a,
        "record changed when closed_claim_ids were reordered"
    );
}

/// A9 (added) — the refusal is deterministic when a ticket is wrong in several ways at once.
#[test]
fn a9_validation_order_is_fixed() {
    // Wrong node AND drifted checkpoint AND already redeemed: the node check must win.
    let t = ticket("sha256:bbbb", 60_000, 1_000, DIGEST);
    let mut i = input("sha256:cccc", 2_000, 0, true, "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");
    i.redeemed_ticket_ids.push(t.ticket_id.clone());
    assert!(matches!(
        validate_ticket(&t, &i),
        Err(HandoffError::NodeMismatch { .. })
    ));

    // Right node, unresolvable target AND drifted checkpoint: the target check must win.
    let t = ticket("node-b", 60_000, 1_000, DIGEST);
    let i = input("node-b", 2_000, 0, false, "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");
    assert!(matches!(
        validate_ticket(&t, &i),
        Err(HandoffError::TargetUnresolved { .. })
    ));

    // Right node, resolvable target, drifted checkpoint AND redeemed: drift must win.
    let i = input("node-b", 2_000, 0, true, "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");
    let mut i2 = i;
    i2.redeemed_ticket_ids.push(t.ticket_id.clone());
    assert!(matches!(
        validate_ticket(&t, &i2),
        Err(HandoffError::CheckpointDrift { .. })
    ));
}

/// A7 (store layer) and A3's row-count half need two real ledgers, so they are declared here and
/// left unimplemented until the design is approved; they belong in
/// `crates/awr-store/tests/device_handoff.rs` (design §4, §5).
#[test]
#[ignore = "TODO: A7 needs migration 015 + a second store (design §3.4/§4); not implemented"]
fn a7_no_orphan_and_no_double_live_claim_across_two_stores() {
    todo!("requires two stores and migration 015; see design §4 A7")
}

/// Keeps the audit-only fields meaningful: they must survive into the record even though the rule
/// never reads them (design §3.2/§3.3).
#[test]
fn sender_clock_is_recorded_but_never_used() {
    let t = ticket("node-b", 60_000, 4_242, DIGEST);
    let record: Value = canonical_ticket_json(&t);
    assert_eq!(record["sender_clock_ms"], serde_json::json!(4_242));
    assert_eq!(record["remaining_ms"], serde_json::json!(60_000));
}
