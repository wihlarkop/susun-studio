use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use uuid::Uuid;

/// Time-to-live for a stream ticket. Short by design: a ticket only has to
/// survive the round trip from "issued" to "EventSource opened".
const TICKET_TTL_MS: i64 = 30_000;

struct TicketEntry {
    scope: String,
    payload: Option<String>,
    expires_at_ms: i64,
}

/// Short-lived, single-use, scope-bound tickets that authorize one SSE stream.
///
/// The daemon's long-lived auth token never appears in a URL: an authenticated
/// POST issues a random ticket, and the `EventSource` URL carries only that
/// ticket. If it leaks, the blast radius is one stream for at most
/// `TICKET_TTL_MS`. A scope is an opaque string such as `job:{id}` or
/// `logs:{project_id}` that binds the ticket to exactly one stream target.
#[derive(Default)]
pub struct StreamTickets {
    tickets: DashMap<String, TicketEntry>,
}

impl StreamTickets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Issues a ticket authorizing a stream for `scope`. Returns the opaque
    /// ticket and its absolute expiry in epoch milliseconds.
    pub fn issue(&self, scope: String) -> (String, i64) {
        self.insert(scope, None)
    }

    /// Issues a ticket that also carries an opaque request payload, so the
    /// follow-up GET (which cannot safely carry a body) can recover it.
    pub fn issue_with_payload(&self, scope: String, payload: String) -> (String, i64) {
        self.insert(scope, Some(payload))
    }

    fn insert(&self, scope: String, payload: Option<String>) -> (String, i64) {
        let ticket = Uuid::new_v4().simple().to_string();
        let expires_at_ms = now_ms().saturating_add(TICKET_TTL_MS);
        self.tickets.insert(
            ticket.clone(),
            TicketEntry {
                scope,
                payload,
                expires_at_ms,
            },
        );
        self.sweep_expired();
        (ticket, expires_at_ms)
    }

    /// Validates and consumes a ticket (single use). `Some(payload)` only when
    /// the ticket exists, is unexpired, and was issued for `scope`.
    pub fn consume(&self, ticket: &str, scope: &str) -> Option<Option<String>> {
        match self.tickets.remove(ticket) {
            Some((_, entry)) if entry.scope == scope && entry.expires_at_ms >= now_ms() => {
                Some(entry.payload)
            }
            _ => None,
        }
    }

    fn sweep_expired(&self) {
        let now = now_ms();
        self.tickets.retain(|_, entry| entry.expires_at_ms >= now);
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
