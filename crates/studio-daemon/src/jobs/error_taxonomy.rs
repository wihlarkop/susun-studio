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

/// Classifies a `susun::BuildError` into a stable code plus a bounded,
/// user-facing summary. Unlike [`classify_error`], this matches on the real
/// typed variants rather than string substrings — `BuildError`'s own
/// `Display` impl can legitimately interpolate a raw host path (`Launch`'s
/// `program`, or the underlying `io::Error` in `source`), so it is never
/// forwarded to the caller; only these fixed, hand-authored messages are.
pub fn classify_build_error(error: &susun::BuildError) -> (&'static str, &'static str) {
    match error {
        susun::BuildError::Cancelled => ("cancelled", "The build was cancelled."),
        susun::BuildError::UnsupportedCapability { .. } => (
            "capability_unsupported",
            "This build requires a capability the engine does not support.",
        ),
        susun::BuildError::InvalidInput { .. } => {
            ("invalid_build_input", "The build request was invalid.")
        }
        susun::BuildError::Launch { .. } => (
            "provider_launch_failed",
            "The build process could not be started.",
        ),
        susun::BuildError::ProcessFailed { .. } => {
            ("build_process_failed", "The build process failed.")
        }
        susun::BuildError::MissingImageIdentity => (
            "missing_image_identity",
            "The build completed without producing a usable image reference.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_error_recognizes_known_substrings() {
        assert_eq!(classify_error("engine operation cancelled"), "cancelled");
        assert_eq!(
            classify_error("engine endpoint unavailable: pipe closed"),
            "engine_unavailable"
        );
        assert_eq!(
            classify_error("engine authentication failed"),
            "permission_error"
        );
        assert_eq!(classify_error("request timed out"), "timeout");
        assert_eq!(classify_error("something unrecognized"), "internal");
    }

    /// Every `BuildError` variant must classify to a stable code, and — the
    /// property this whole function exists to guarantee — the returned
    /// message must never be the variant's own raw `Display` text, since
    /// `Launch`/`ProcessFailed` (and potentially others via their `source`)
    /// can legitimately contain a raw host path or OS error string.
    #[test]
    fn classify_build_error_never_forwards_the_raw_display_text() {
        let cases: Vec<(susun::BuildError, &str)> = vec![
            (susun::BuildError::Cancelled, "cancelled"),
            (
                susun::BuildError::UnsupportedCapability {
                    capability: "secrets",
                },
                "capability_unsupported",
            ),
            (
                susun::BuildError::InvalidInput {
                    detail: r"C:\Users\someone\secret-project\.env leaked".to_owned(),
                },
                "invalid_build_input",
            ),
            (
                susun::BuildError::Launch {
                    program: std::path::PathBuf::from(r"C:\Users\someone\AppData\docker.exe"),
                    source: std::io::Error::other("boom"),
                },
                "provider_launch_failed",
            ),
            (
                susun::BuildError::ProcessFailed {
                    status: "exit code 1".to_owned(),
                },
                "build_process_failed",
            ),
            (
                susun::BuildError::MissingImageIdentity,
                "missing_image_identity",
            ),
        ];

        for (error, expected_code) in cases {
            let raw_display = error.to_string();
            let (code, message) = classify_build_error(&error);
            assert_eq!(code, expected_code);
            assert_ne!(
                message, raw_display,
                "classify_build_error must never forward the raw Display text for {code}"
            );
            assert!(
                !message.to_lowercase().contains("users") && !message.contains(r"C:\"),
                "classify_build_error leaked a path-shaped fragment for {code}: {message}"
            );
        }
    }
}
