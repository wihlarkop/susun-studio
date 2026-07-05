//! Best-effort classification of redacted job error messages into a stable
//! code the UI can key off of. susun redacts per-action errors to plain
//! strings by design (see `ActionExecutionResult.error`), so there is no
//! richer type left to preserve by the time an error reaches here — this
//! matches known `EngineError`/`EngineConnectionError` `Display` substrings
//! rather than doing type-safe classification.

/// Classifies a redacted error message. Defaults to `"internal"` when
/// nothing recognizable matches.
pub fn classify_error(message: &str) -> &'static str {
    if message.contains("engine operation cancelled") {
        return "cancelled";
    }
    if message.contains("engine endpoint unavailable")
        || message.contains("engine TLS configuration failed")
        || message.contains("engine API negotiation failed")
    {
        return "engine_unavailable";
    }
    if message.contains("engine authentication failed") {
        return "permission_error";
    }
    if message.contains("engine resource conflict")
        || message.contains("engine resource not found")
        || message.contains("engine does not support")
    {
        return "user_error";
    }
    if message.to_lowercase().contains("timed out") || message.to_lowercase().contains("timeout") {
        return "timeout";
    }
    "internal"
}
