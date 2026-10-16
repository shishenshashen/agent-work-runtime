//! Cross-device session handoff — design FEAT-XDH-001 v1.1 §3.2–§3.4.
//!
//! ## Contract (revised after design review v1.1)
//!
//! This feature **does not cancel** clock skew between two devices. There is no shared arbiter
//! time in the local domain, so `expires_at_ms` computed on the sender is meaningless as an
//! absolute instant on the receiver. What the rule guarantees instead is that skew is **never
//! amplified by the transfer**:
//!
//! 1. a ticket may not overstate its budget: `remaining_ms <= sender_declared_ttl_ms - MARGIN_MS`,
//!    otherwise [`HandoffError::TicketOverstated`];
//! 2. the receiver rebuilds the deadline on **its own** clock and may only shorten it:
//!    `local_deadline_ms = receiver_at_ms + remaining_ms - rtt_ms`, so
//!    `local_deadline_ms <= receiver_at_ms + sender_declared_ttl_ms - MARGIN_MS - rtt_ms`;
//! 3. `sender_clock_ms` is evidence only. The rule never reads it, and A2b proves the test suite
//!    can detect an implementation that does.
//!
//! `crates/awr-team-pg/src/scoped_runner.rs:119` keeps a *local effect budget* monotone in the
//! server's lease; it is **not** a precedent for cancelling skew, and the wording there must not be
//! reused to claim that this module does.

use crate::error::Error;
use serde_json::json;
use sha2::{Digest, Sha256};

/// Slack between what the sender declared it was granted and what it may forward.
///
/// A ticket that claims more remaining time than `declared_ttl - MARGIN_MS` is refused rather than
/// trusted: the margin is the sender's own round trip plus scheduling slop, and it is the only
/// place where this design bounds skew instead of hiding it.
pub const MARGIN_MS: i64 = 1_000;

/// A portable handoff ticket (design §3.2). `work_key` and `branch_name` are the search keys,
/// because the ULID spaces of two devices do not intersect; every ULID field is audit-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffTicket {
    pub ticket_id: String,
    pub work_key: String,
    pub branch_name: String,
    pub from_node_id: String,
    pub to_node_id: String,
    pub sender_session_id: String,
    pub checkpoint_id: String,
    pub checkpoint_digest: String,
    /// Sender's own wall clock. Evidence only: the rule must never read it (§3.2/§3.3).
    pub sender_clock_ms: i64,
    pub issued_at_ms: i64,
    /// The TTL the sender was originally granted, on its own clock.
    pub sender_declared_ttl_ms: i64,
    /// What the sender believes is left: `deadline - sender_clock`, bounded by (1) above.
    pub remaining_ms: i64,
    pub closed_claim_ids: Vec<String>,
}

/// Receiver-side facts. Nothing from the sender is trusted except `remaining_ms`, which (1) bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedeemInput {
    pub receiver_node_id: String,
    pub receiver_at_ms: i64,
    pub rtt_ms: i64,
    pub target_resolved: bool,
    pub local_checkpoint_digest: String,
    pub redeemed_ticket_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HandoffError {
    #[error("handoff ticket is addressed to {expected}, not {got}")]
    NodeMismatch { expected: String, got: String },
    #[error("handoff target {work_key} on {branch_name} does not resolve locally")]
    TargetUnresolved { work_key: String, branch_name: String },
    #[error("checkpoint drifted: ticket expects {expected}, local is {got}")]
    CheckpointDrift { expected: String, got: String },
    #[error("handoff ticket {ticket_id} was already redeemed")]
    TicketAlreadyRedeemed { ticket_id: String },
    #[error("ticket claims {remaining_ms}ms remaining but only {ceiling_ms}ms is admissible")]
    TicketOverstated { remaining_ms: i64, ceiling_ms: i64 },
    #[error("handoff ticket has no remaining budget ({remaining_ms}ms)")]
    ClaimExpired { remaining_ms: i64 },
    #[error("handoff ticket carries no checkpoint payload, so the digest cannot be checked on this device")]
    CheckpointUnverifiable,
    #[error("handoff record could not be serialised: {0}")]
    Record(String),
}

impl From<HandoffError> for Error {
    fn from(value: HandoffError) -> Self {
        Error::InvalidInput(value.to_string())
    }
}

/// The greatest `remaining_ms` a ticket may forward for a given declared TTL.
pub fn remaining_ceiling_ms(sender_declared_ttl_ms: i64) -> i64 {
    sender_declared_ttl_ms.saturating_sub(MARGIN_MS)
}

/// Redemption validation, in the fixed order of design §3.4. The order is part of the contract: a
/// ticket wrong in several ways must fail the same way every time.
pub fn validate_ticket(ticket: &HandoffTicket, input: &RedeemInput) -> Result<(), HandoffError> {
    if ticket.to_node_id != input.receiver_node_id {
        return Err(HandoffError::NodeMismatch {
            expected: ticket.to_node_id.clone(),
            got: input.receiver_node_id.clone(),
        });
    }
    if !input.target_resolved {
        return Err(HandoffError::TargetUnresolved {
            work_key: ticket.work_key.clone(),
            branch_name: ticket.branch_name.clone(),
        });
    }
    if input.local_checkpoint_digest != ticket.checkpoint_digest {
        return Err(HandoffError::CheckpointDrift {
            expected: ticket.checkpoint_digest.clone(),
            got: input.local_checkpoint_digest.clone(),
        });
    }
    if input
        .redeemed_ticket_ids
        .iter()
        .any(|id| id == &ticket.ticket_id)
    {
        return Err(HandoffError::TicketAlreadyRedeemed {
            ticket_id: ticket.ticket_id.clone(),
        });
    }
    let ceiling = remaining_ceiling_ms(ticket.sender_declared_ttl_ms);
    if ticket.remaining_ms > ceiling {
        return Err(HandoffError::TicketOverstated {
            remaining_ms: ticket.remaining_ms,
            ceiling_ms: ceiling,
        });
    }
    Ok(())
}

/// Clock normalisation (design §3.3): the receiver rebuilds the deadline on its own clock and may
/// only **shorten** it, never extend it. Call [`validate_ticket`] first; this function is the
/// arithmetic half of the contract.
pub fn local_deadline_ms(
    ticket: &HandoffTicket,
    receiver_at_ms: i64,
    rtt_ms: i64,
) -> Result<i64, HandoffError> {
    if ticket.remaining_ms <= 0 {
        return Err(HandoffError::ClaimExpired {
            remaining_ms: ticket.remaining_ms,
        });
    }
    let budget = ticket.remaining_ms.saturating_sub(rtt_ms);
    Ok(receiver_at_ms.saturating_add(budget))
}

/// Canonical, byte-stable ticket JSON: sorted keys, integers only, `closed_claim_ids` sorted, so
/// two devices can compare a ticket by digest.
pub fn canonical_ticket_json(ticket: &HandoffTicket) -> Result<String, HandoffError> {
    let mut closed = ticket.closed_claim_ids.clone();
    closed.sort();
    let value = json!({
        "branch_name": &ticket.branch_name,
        "checkpoint_digest": &ticket.checkpoint_digest,
        "checkpoint_id": &ticket.checkpoint_id,
        "closed_claim_ids": closed,
        "from_node_id": &ticket.from_node_id,
        "issued_at_ms": ticket.issued_at_ms,
        "remaining_ms": ticket.remaining_ms,
        "sender_clock_ms": ticket.sender_clock_ms,
        "sender_declared_ttl_ms": ticket.sender_declared_ttl_ms,
        "sender_session_id": &ticket.sender_session_id,
        "ticket_id": &ticket.ticket_id,
        "to_node_id": &ticket.to_node_id,
        "work_key": &ticket.work_key,
    });
    serde_json::to_string(&value).map_err(|error| HandoffError::Record(error.to_string()))
}

/// Lowercase hex sha256 of [`canonical_ticket_json`].
pub fn canonical_ticket_digest(ticket: &HandoffTicket) -> Result<String, HandoffError> {
    let record = canonical_ticket_json(ticket)?;
    Ok(Sha256::digest(record.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

// ---------------------------------------------------------------------------
// Cross-device payload verification (design §3.5, acceptance A11/A12/A14).
//
// The receiving device has no row for the sender's checkpoint: the two devices do not share a
// `.awr/state.db`, and `checkpoints` is keyed by ULIDs that do not intersect. So a ticket that
// carries only `checkpoint_id` + `checkpoint_digest` cannot be checked at all — trusting the
// sender's digest would make `CheckpointDrift` vacuously true.
//
// What makes the check real is that the payload travels INSIDE the record (see migration 015's
// `checkpoint_payload_json`) and the receiver re-hashes the bytes it actually holds. The sender's
// digest is an expectation, never a proof. This module keeps no payload field on `HandoffTicket`
// so the ticket's canonical digest stays exactly what v1.1 froze; the payload is carried by the
// record wrapper below and by the store row.
// ---------------------------------------------------------------------------

/// The full cross-device record: the portable ticket plus the checkpoint bytes it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffRecord {
    pub ticket: HandoffTicket,
    /// Canonical checkpoint bytes, exactly as the sender read them. Never re-serialised in transit.
    pub checkpoint_payload_json: String,
}

/// Lowercase hex sha256 of exactly the bytes that travelled.
pub fn payload_digest(payload_bytes: &[u8]) -> String {
    Sha256::digest(payload_bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Receiver-side gate that keeps `CheckpointDrift` reachable across devices (A11/A12).
///
/// - no payload / empty payload / malformed expected digest -> [`HandoffError::CheckpointUnverifiable`]
///   (never `Ok`, and never reported as drift: "cannot check" is not "checked and equal")
/// - payload whose recomputed digest differs -> [`HandoffError::CheckpointDrift`]
/// - otherwise -> `Ok(recomputed_digest)`, which the caller must be able to compare byte-for-byte
///   with the digest it stores next to the materialised payload.
pub fn verify_payload(
    payload_bytes: Option<&[u8]>,
    expected_digest: &str,
) -> Result<String, HandoffError> {
    let bytes = match payload_bytes {
        Some(bytes) if !bytes.is_empty() => bytes,
        _ => return Err(HandoffError::CheckpointUnverifiable),
    };
    let well_formed = expected_digest.len() == 64
        && expected_digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !well_formed {
        return Err(HandoffError::CheckpointUnverifiable);
    }
    let actual = payload_digest(bytes);
    if actual != expected_digest {
        return Err(HandoffError::CheckpointDrift {
            expected: expected_digest.to_string(),
            got: actual,
        });
    }
    Ok(actual)
}

// ---------------------------------------------------------------------------
// Counterexamples. These exist ONLY so the acceptance tests can prove they can
// fail (A2b, the A3b pattern from the claim-arbitration work). Nothing in the
// store, runtime or CLI may call them.
// ---------------------------------------------------------------------------

/// A skew-**amplifying** implementation: it anchors the deadline on the sender's wall clock, so a
/// sender whose clock runs fast hands the receiver a longer lease than it was ever granted.
///
/// Deliberately wrong. Used by A2b.
pub fn local_deadline_ms_naive_sender_clock(
    ticket: &HandoffTicket,
    _receiver_at_ms: i64,
    _rtt_ms: i64,
) -> i64 {
    ticket.sender_clock_ms.saturating_add(ticket.remaining_ms)
}

/// An implementation that ignores the forwarded budget and re-grants the full declared TTL,
/// silently **extending** the lease across the transfer. Deliberately wrong. Used by A2b.
pub fn local_deadline_ms_naive_extend(
    ticket: &HandoffTicket,
    receiver_at_ms: i64,
    _rtt_ms: i64,
) -> i64 {
    receiver_at_ms.saturating_add(ticket.sender_declared_ttl_ms)
}
