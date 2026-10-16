-- Cross-device session handoff (FEAT-XDH-001, migration 15).
--
-- Two devices do not share ULID spaces, so a handoff ticket addresses its target by the *authored*
-- names (work external_key, branch name) and names the receiving device by node_id. That is the
-- whole reason this table exists: before it, a handoff named a session id that only the sending
-- device could resolve.
--
-- The checkpoint payload travels INSIDE the ticket. The receiving device has no row for the
-- sender's checkpoint, so "verify the digest locally" is only meaningful once the payload has been
-- materialised there; carrying the payload is what makes the drift check (A5) meaningful rather
-- than impossible across devices.
--
-- Redemption is exactly once per (project_id, ticket_id). The final CHECK keeps the two redemption
-- columns in agreement, so a half-redeemed ticket cannot be written at all.
--
-- The margin in the remaining_ms CHECK must stay equal to awr_core::device_handoff::MARGIN_MS
-- (1000). The store test asserts both directions: an overstated row is rejected here, and the pure
-- rule rejects the same row with HandoffTicketOverstated.

ALTER TABLE sessions ADD COLUMN node_id TEXT NOT NULL DEFAULT 'local';
ALTER TABLE claims ADD COLUMN node_id TEXT NOT NULL DEFAULT 'local';

CREATE TABLE handoff_tickets (
    project_id TEXT NOT NULL REFERENCES projects(id),
    ticket_id TEXT NOT NULL,
    work_item_id TEXT NOT NULL,
    branch_id TEXT,
    from_node_id TEXT NOT NULL,
    to_node_id TEXT NOT NULL,
    sender_session_id TEXT NOT NULL,
    sender_checkpoint_id TEXT NOT NULL,
    checkpoint_payload_json TEXT NOT NULL CHECK (json_valid(checkpoint_payload_json)),
    checkpoint_digest TEXT NOT NULL CHECK (length(checkpoint_digest) = 64),
    sender_clock_ms INTEGER NOT NULL,
    issued_at_ms INTEGER NOT NULL,
    sender_declared_ttl_ms INTEGER NOT NULL CHECK (sender_declared_ttl_ms > 0),
    remaining_ms INTEGER NOT NULL CHECK (remaining_ms <= sender_declared_ttl_ms - 1000),
    closed_claim_ids_json TEXT NOT NULL CHECK (json_valid(closed_claim_ids_json)),
    record_json TEXT NOT NULL CHECK (json_valid(record_json)),
    record_sha256 TEXT NOT NULL CHECK (length(record_sha256) = 64),
    exported_at_ms INTEGER NOT NULL,
    redeemed_at_ms INTEGER,
    receiver_session_id TEXT,
    receiver_node_id TEXT,
    PRIMARY KEY (project_id, ticket_id),
    FOREIGN KEY (project_id, work_item_id) REFERENCES work_items(project_id, id),
    FOREIGN KEY (project_id, branch_id) REFERENCES branches(project_id, id),
    FOREIGN KEY (project_id, sender_session_id) REFERENCES sessions(project_id, id),
    FOREIGN KEY (project_id, receiver_session_id) REFERENCES sessions(project_id, id),
    CHECK ((redeemed_at_ms IS NULL) = (receiver_session_id IS NULL))
);

CREATE INDEX handoff_tickets_pending
    ON handoff_tickets(project_id, to_node_id, redeemed_at_ms);
CREATE INDEX handoff_tickets_sender
    ON handoff_tickets(project_id, sender_session_id, exported_at_ms);
