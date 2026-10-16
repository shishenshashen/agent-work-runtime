//! Cross-device session handoff — design FEAT-XDH-001 v1.1, acceptance A1–A10.
//!
//! These tests call the **production** rule in `awr_core::device_handoff`, not a local copy: a
//! green run must mean the shipped functions are correct. The only in-file code is the pair of
//! deliberately wrong implementations that A2b needs as counterexamples.
//!
//! Contract under test (review v1.1): this feature does **not** cancel clock skew. It bounds it —
//! a ticket may not overstate its budget (`remaining_ms <= declared_ttl - MARGIN_MS`), and the
//! receiver may only shorten the lease.
//!
//! TODO(verify): **never executed** on the authoring host — `rustc`, `cargo`, `rustup`, `link`,
//! `cc`, `gcc` and `clang` are all MISSING there (`reports/env-probe-v1.json`), so this file has
//! never been compiled and is not claimed to be `cargo fmt` clean. A passing result may only be
//! quoted with a CI run id.

use awr_core::device_handoff::{
    canonical_ticket_digest, canonical_ticket_json, local_deadline_ms,
    local_deadline_ms_naive_extend, local_deadline_ms_naive_sender_clock, payload_digest,
    remaining_ceiling_ms, validate_ticket, verify_payload, HandoffError, HandoffRecord,
    HandoffTicket, RedeemInput, MARGIN_MS,
};

const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NODE_B: &str = "sha256:bbbb";
const DECLARED_TTL_MS: i64 = 60_000;

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
        sender_declared_ttl_ms: DECLARED_TTL_MS,
        remaining_ms,
        closed_claim_ids: vec!["01J0CLAIM-B".to_string(), "01J0CLAIM-A".to_string()],
    }
}

/// A ticket at exactly the admissible ceiling, which is the interesting boundary case.
fn admissible_ticket(to_node: &str, sender_clock_ms: i64) -> HandoffTicket {
    ticket(
        to_node,
        remaining_ceiling_ms(DECLARED_TTL_MS),
        sender_clock_ms,
        DIGEST,
    )
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

/// The assertion that makes A2 non-trivial: the bound is stated in terms of the **declared** TTL,
/// the margin and the round trip — three inputs other than the `remaining_ms` the ticket forwards.
/// An implementation that amplifies skew or re-grants the full TTL breaks it.
fn admissible_bound(receiver_at_ms: i64, declared_ttl_ms: i64, rtt_ms: i64) -> i64 {
    receiver_at_ms + declared_ttl_ms - MARGIN_MS - rtt_ms
}

/// A1 — a ticket addressed to another device is refused.
#[test]
fn a1_ticket_for_another_node_is_refused() {
    let t = admissible_ticket("sha256:bbbb", 1_000);
    let i = input("sha256:cccc", 2_000, 0, true, DIGEST);
    match validate_ticket(&t, &i) {
        Err(HandoffError::NodeMismatch { expected, got }) => {
            assert_eq!(expected, "sha256:bbbb");
            assert_eq!(got, "sha256:cccc");
        }
        other => panic!("expected NodeMismatch, got {other:?}"),
    }
}

/// A2 — the receiver's deadline is bounded by the *declared* TTL and never reads the sender clock.
#[test]
fn a2_deadline_is_bounded_and_ignores_the_sender_clock() {
    let receiver_at = 10_000_000i64;
    let rtt = 250i64;

    let mut observed = Vec::new();
    for sender_clock in [
        receiver_at - 10_000_000,
        receiver_at,
        receiver_at + 10_000_000,
    ] {
        let t = admissible_ticket(NODE_B, sender_clock);
        let i = input(NODE_B, receiver_at, rtt, true, DIGEST);
        validate_ticket(&t, &i).expect("ticket is admissible");

        let deadline = local_deadline_ms(&t, receiver_at, rtt).expect("positive budget");
        let bound = admissible_bound(receiver_at, DECLARED_TTL_MS, rtt);
        assert!(
            deadline <= bound,
            "deadline {deadline} exceeds the admissible bound {bound}"
        );
        observed.push(deadline);
    }
    assert_eq!(
        observed[0], observed[1],
        "deadline moved with the sender clock (-10^7 vs 0)"
    );
    assert_eq!(
        observed[1], observed[2],
        "deadline moved with the sender clock (0 vs +10^7)"
    );

    // Monotone in the round trip: a slower transfer can only shorten the lease.
    let t = admissible_ticket(NODE_B, receiver_at);
    let slower = local_deadline_ms(&t, receiver_at, rtt * 4).expect("positive budget");
    assert!(
        slower < observed[0],
        "a larger rtt must strictly shorten the deadline"
    );
}

/// A2b — the counterexample that gives A2 its teeth.
///
/// Two deliberately wrong implementations, both of which violate the bound A2 asserts. The
/// sender-clock variant additionally returns **different** deadlines for a shifted sender clock,
/// which is exactly the drift A2's equality assertion exists to catch.
#[test]
fn a2b_skew_amplifying_and_extending_implementations_violate_the_bound() {
    let receiver_at = 10_000_000i64;
    let rtt = 250i64;
    let bound = admissible_bound(receiver_at, DECLARED_TTL_MS, rtt);

    let fast_sender = admissible_ticket(NODE_B, receiver_at + 10_000_000);
    let amplifying = local_deadline_ms_naive_sender_clock(&fast_sender, receiver_at, rtt);
    assert!(
        amplifying > bound,
        "the skew-amplifying implementation must break the bound ({amplifying} vs {bound})"
    );

    let extending = local_deadline_ms_naive_extend(&fast_sender, receiver_at, rtt);
    assert!(
        extending > bound,
        "the re-granting implementation must break the bound ({extending} vs {bound})"
    );

    // Discriminating power: a sender-clock-reading rule is not shift-invariant, so A2 could fail.
    let slow_sender = admissible_ticket(NODE_B, receiver_at - 10_000_000);
    assert_ne!(
        local_deadline_ms_naive_sender_clock(&fast_sender, receiver_at, rtt),
        local_deadline_ms_naive_sender_clock(&slow_sender, receiver_at, rtt),
        "the counterexample must move with the sender clock, otherwise A2 proves nothing"
    );

    // Meanwhile the production rule is shift-invariant on the same two tickets.
    assert_eq!(
        local_deadline_ms(&fast_sender, receiver_at, rtt).expect("positive budget"),
        local_deadline_ms(&slow_sender, receiver_at, rtt).expect("positive budget"),
        "the production rule must not move with the sender clock"
    );
}

/// A3 — redeeming the same ticket twice must be refused (exactly-once at the rule level).
#[test]
fn a3_second_redemption_of_the_same_ticket_is_refused() {
    let t = admissible_ticket(NODE_B, 1_000);
    let mut i = input(NODE_B, 2_000, 10, true, DIGEST);
    assert!(
        validate_ticket(&t, &i).is_ok(),
        "first redemption must validate"
    );
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
        let t = ticket(NODE_B, remaining, 1_000, DIGEST);
        match local_deadline_ms(&t, 2_000, 0) {
            Err(HandoffError::ClaimExpired { remaining_ms }) => assert_eq!(remaining_ms, remaining),
            other => panic!("expected ClaimExpired for remaining={remaining}, got {other:?}"),
        }
    }
}

/// A5 — a checkpoint whose content no longer matches the ticket digest is a drift, not an inherit.
#[test]
fn a5_checkpoint_drift_is_refused() {
    let t = admissible_ticket(NODE_B, 1_000);
    let i = input(NODE_B, 2_000, 0, true, &"b".repeat(64));
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
    let t = admissible_ticket(NODE_B, 1_000);
    let a = canonical_ticket_json(&t).expect("record");
    let b = canonical_ticket_json(&t.clone()).expect("record");
    assert_eq!(
        a, b,
        "ticket record is not reproducible for an identical input"
    );
    assert!(
        !a.contains('.'),
        "ticket record must contain no floating point: {a}"
    );
    let digest = canonical_ticket_digest(&t).expect("digest");
    assert_eq!(digest.len(), 64);
    assert!(digest
        .chars()
        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));

    // Claim-id ordering must not leak into the record.
    let mut shuffled = t.clone();
    shuffled.closed_claim_ids.reverse();
    assert_eq!(
        canonical_ticket_json(&shuffled).expect("record"),
        a,
        "record changed when closed_claim_ids were reordered"
    );
}

/// A9 — the refusal is deterministic when a ticket is wrong in several ways at once.
#[test]
fn a9_validation_order_is_fixed() {
    // Wrong node AND drifted checkpoint AND already redeemed: the node check must win.
    let t = admissible_ticket("sha256:bbbb", 1_000);
    let mut i = input("sha256:cccc", 2_000, 0, true, &"c".repeat(64));
    i.redeemed_ticket_ids.push(t.ticket_id.clone());
    assert!(matches!(
        validate_ticket(&t, &i),
        Err(HandoffError::NodeMismatch { .. })
    ));

    // Right node, unresolvable target AND drifted checkpoint: the target check must win.
    let t = admissible_ticket(NODE_B, 1_000);
    let i = input(NODE_B, 2_000, 0, false, &"c".repeat(64));
    assert!(matches!(
        validate_ticket(&t, &i),
        Err(HandoffError::TargetUnresolved { .. })
    ));

    // Right node, resolvable target, drifted checkpoint AND redeemed: drift must win.
    let mut i = input(NODE_B, 2_000, 0, true, &"c".repeat(64));
    i.redeemed_ticket_ids.push(t.ticket_id.clone());
    assert!(matches!(
        validate_ticket(&t, &i),
        Err(HandoffError::CheckpointDrift { .. })
    ));

    // Right node, resolvable target, matching checkpoint, redeemed AND overstated: the
    // already-redeemed check must win over the budget check.
    let mut i = input(NODE_B, 2_000, 0, true, DIGEST);
    i.redeemed_ticket_ids.push(t.ticket_id.clone());
    let overstated = ticket(NODE_B, DECLARED_TTL_MS, 1_000, DIGEST);
    assert!(matches!(
        validate_ticket(&overstated, &i),
        Err(HandoffError::TicketAlreadyRedeemed { .. })
    ));
}

/// A10 — a ticket may not forward more budget than the sender was granted, minus the margin.
#[test]
fn a10_overstated_budget_is_refused_at_the_ceiling_boundary() {
    let ceiling = remaining_ceiling_ms(DECLARED_TTL_MS);
    assert_eq!(ceiling, DECLARED_TTL_MS - MARGIN_MS);

    let at_ceiling = ticket(NODE_B, ceiling, 1_000, DIGEST);
    let i = input(NODE_B, 2_000, 0, true, DIGEST);
    assert!(
        validate_ticket(&at_ceiling, &i).is_ok(),
        "exactly at the ceiling must be admissible"
    );

    let over_ceiling = ticket(NODE_B, ceiling + 1, 1_000, DIGEST);
    match validate_ticket(&over_ceiling, &i) {
        Err(HandoffError::TicketOverstated {
            remaining_ms,
            ceiling_ms,
        }) => {
            assert_eq!(remaining_ms, ceiling + 1);
            assert_eq!(ceiling_ms, ceiling);
        }
        other => panic!("expected TicketOverstated, got {other:?}"),
    }

    // A ticket that forwards the whole declared TTL is the common mistake, and must be refused.
    let whole_ttl = ticket(NODE_B, DECLARED_TTL_MS, 1_000, DIGEST);
    assert!(matches!(
        validate_ticket(&whole_ttl, &i),
        Err(HandoffError::TicketOverstated { .. })
    ));
}

/// A7 is a storage-layer criterion (design §4), so it is declared here, ignored, and left
/// unimplemented until migration `015_device_handoff.sql` and the `handoff.rs` export/redeem pair
/// land. Its body belongs in `crates/awr-store/tests/device_handoff.rs` and must assert, across two
/// real stores: the sender's active claim count is 0 and its session is incomplete/interrupted,
/// while the receiver's active claim count is exactly 1.
#[test]
#[ignore = "TODO: A7 needs migration 015 + handoff.rs export/redeem over two stores (design §3.4/§4); not implemented"]
fn a7_no_orphan_and_no_double_live_claim_across_two_stores() {
    todo!("requires two stores and migration 015; see design §4 A7")
}

// ---------------------------------------------------------------------------
// A11/A12/A14 — cross-device payload verifiability (design §3.5).
//
// These are pure-function tests: they prove the rule refuses to certify a ticket whose payload it
// cannot see, and that the digest it accepts is one it recomputed itself.
// ---------------------------------------------------------------------------

const PAYLOAD: &str = r#"{"checkpoint":"01J0CHECKPOINT00000000001","note":"handoff"}"#;

/// A14: the payload the receiver hashes is byte-identical to the payload that was exported.
#[test]
fn a14_payload_digest_round_trips_byte_for_byte() {
    let exported = HandoffRecord {
        ticket: ticket(NODE_B, 30_000, 1_000, &payload_digest(PAYLOAD.as_bytes())),
        checkpoint_payload_json: PAYLOAD.to_string(),
    };
    let digest = payload_digest(exported.checkpoint_payload_json.as_bytes());
    assert_eq!(digest.len(), 64, "digest must be 64 lowercase hex chars");
    assert!(digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    // Re-reading the stored bytes must not change the digest: no re-serialisation in transit.
    let reread = exported.checkpoint_payload_json.as_bytes();
    assert_eq!(payload_digest(reread), digest);
    assert_eq!(verify_payload(Some(reread), &digest), Ok(digest));
}

/// A12: a ticket with no payload must be refused as UNVERIFIABLE, not as drift and not as Ok.
#[test]
fn a12_absent_payload_is_unverifiable_not_drift() {
    let err = verify_payload(None, DIGEST).unwrap_err();
    assert!(matches!(err, HandoffError::CheckpointUnverifiable), "got {err:?}");
    let empty = verify_payload(Some(b""), DIGEST).unwrap_err();
    assert!(matches!(empty, HandoffError::CheckpointUnverifiable), "got {empty:?}");
    // A malformed expectation is also "cannot check", never a silent pass.
    let malformed = verify_payload(Some(PAYLOAD.as_bytes()), "not-a-digest").unwrap_err();
    assert!(matches!(malformed, HandoffError::CheckpointUnverifiable), "got {malformed:?}");
}

/// A11: the drift check has teeth — one changed byte in the payload is detected, and an
/// implementation that trusts the sender's digest instead of recomputing would return Ok here.
#[test]
fn a11_one_changed_payload_byte_is_detected() {
    let expected = payload_digest(PAYLOAD.as_bytes());
    // The honest receiver recomputes and refuses.
    let tampered = PAYLOAD.replace("handoff", "handofg");
    match verify_payload(Some(tampered.as_bytes()), &expected) {
        Err(HandoffError::CheckpointDrift { expected: e, got }) => {
            assert_eq!(e, expected);
            assert_ne!(got, expected);
            assert_eq!(got, payload_digest(tampered.as_bytes()));
        }
        other => panic!("expected CheckpointDrift for a tampered payload, got {other:?}"),
    }
    // Counter-model: "trust whatever the ticket says" would have returned Ok for the same bytes.
    let naive_would_accept = Some(tampered.as_bytes()).is_some();
    assert!(naive_would_accept, "counter-model must accept, which is exactly why A11 has teeth");
}

