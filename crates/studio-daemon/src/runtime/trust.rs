//! Trusted executable resolution and identity verification.
//!
//! Absolute-path resolution alone prevents `PATH` shadowing but does not prove
//! the file at that path is the expected executable. This module defines the
//! platform-neutral *policy* — where each [`TrustedProgram`] may live and which
//! signer identities are acceptable — and the pure matching logic that decides
//! whether a resolved path plus its extracted [`SignerFacts`] satisfy that
//! policy. The Windows Authenticode/publisher extraction that produces
//! `SignerFacts` lives in a `#[cfg(windows)]` layer; keeping the policy types
//! platform-neutral lets the matching logic be unit-tested on any host and lets
//! macOS/Linux identity checks be added later without reshaping the contract.
//!
//! Security rules enforced here:
//! - The publisher allowlist is defined in trusted code, never by the UI.
//! - Verification failure yields a typed error and never falls back to
//!   path-only execution or elevation.
//! - Publisher matching uses a pinned certificate thumbprint when available,
//!   otherwise exact (case-insensitive) subject common-name equality — never a
//!   substring test on a display name.

use std::path::{Path, PathBuf};

use super::command::TrustedProgram;

/// Where a trusted program's executable is permitted to live. A resolved path
/// must sit directly in `directory` with file name `file_name`; nothing is
/// trusted merely because it was found on `PATH`.
#[derive(Debug, Clone)]
pub struct TrustedPathRule {
    pub label: &'static str,
    pub directory: PathBuf,
    pub file_name: &'static str,
}

impl TrustedPathRule {
    /// Whether `resolved` is exactly this rule's file in this rule's directory.
    /// Path components are compared case-insensitively so Windows path casing
    /// (`C:\Windows` vs `c:\windows`) does not defeat the rule.
    fn matches(&self, resolved: &Path) -> bool {
        let Some(parent) = resolved.parent() else {
            return false;
        };
        let Some(name) = resolved.file_name() else {
            return false;
        };
        path_eq_ignore_case(parent, &self.directory)
            && name.to_string_lossy().eq_ignore_ascii_case(self.file_name)
    }
}

/// A normalized, trusted publisher identity. Prefer a pinned SHA-1 certificate
/// thumbprint; fall back to exact normalized subject-CN equality where a
/// thumbprint would be too brittle across signing-certificate rotations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublisherIdentity {
    pub label: &'static str,
    pub subject_cn: &'static str,
    /// Uppercase hex SHA-1 thumbprint, no separators. When set, it must match.
    pub thumbprint_sha1: Option<&'static str>,
}

/// The signer facts the platform layer extracts from a binary's Authenticode
/// signature. Platform-neutral so the matching logic is host-independent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerFacts {
    /// Whether the OS trust provider (WinVerifyTrust on Windows) accepted the
    /// certificate chain for the Authenticode policy.
    pub trusted_chain: bool,
    /// Subject common name of the leaf signing certificate.
    pub subject_cn: String,
    /// Uppercase hex SHA-1 thumbprint of the leaf certificate, no separators.
    pub thumbprint_sha1: String,
}

/// A known system binary whose expected publisher is fixed (e.g. `powershell.exe`
/// in System32, always Microsoft-signed).
#[derive(Debug, Clone)]
pub struct SystemBinaryIdentity {
    pub file_name: &'static str,
    pub expected_publisher: PublisherIdentity,
}

/// How strictly to verify the signer of a resolved executable.
#[derive(Debug, Clone)]
pub enum PublisherPolicy {
    /// The signature chain must be trusted and the signer must match one of the
    /// listed publishers.
    Required {
        allowed_publishers: Vec<PublisherIdentity>,
    },
    /// A known system binary: still requires a trusted chain and the expected
    /// publisher, and the resolved file name must match `expected_file`.
    OptionalForKnownSystemBinary { expected_file: SystemBinaryIdentity },
    /// No signer requirement (path trust only). Used only where the platform has
    /// no code-signing story yet; never selected on Windows for a privileged
    /// program.
    #[allow(dead_code)]
    NotApplicable,
}

/// The complete trust policy for one [`TrustedProgram`]: acceptable locations
/// plus the signer requirement.
#[derive(Debug, Clone)]
pub struct TrustedProgramPolicy {
    pub program: TrustedProgram,
    pub allowed_paths: Vec<TrustedPathRule>,
    pub publisher_policy: PublisherPolicy,
}

/// A fully-verified executable: an absolute path that satisfied its program's
/// path and publisher policy. Held in daemon-internal audit data; never
/// surfaced to the UI in raw form.
#[derive(Debug, Clone)]
pub struct VerifiedExecutable {
    pub program: TrustedProgram,
    pub path: PathBuf,
    /// The signer accepted for this executable, or `None` for a `NotApplicable`
    /// publisher policy.
    pub signer: Option<SignerFacts>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TrustError {
    #[error("no trusted install location found for {program}")]
    ExecutableNotFound { program: &'static str },
    #[error("{program} resolved to an untrusted path `{path}`")]
    UntrustedPath { program: &'static str, path: String },
    #[error("{program} at `{path}` is not Authenticode-trusted")]
    ExecutableIdentityMismatch { program: &'static str, path: String },
    #[error("{program} at `{path}` is signed by an unapproved publisher")]
    UntrustedPublisher { program: &'static str, path: String },
    #[error("failed to verify {program}: {detail}")]
    VerificationFailed {
        program: &'static str,
        detail: String,
    },
}

/// Decides whether a resolved path plus its (optional) extracted signer facts
/// satisfy a program's policy. This is the single trust decision point; the
/// platform layer calls it both at resolution time and again immediately before
/// launch (time-of-check/time-of-use protection).
pub fn evaluate_trust(
    policy: &TrustedProgramPolicy,
    resolved_path: &Path,
    signer: Option<&SignerFacts>,
) -> Result<VerifiedExecutable, TrustError> {
    let program = policy.program.name();
    let path_display = resolved_path.to_string_lossy().into_owned();

    if !policy
        .allowed_paths
        .iter()
        .any(|rule| rule.matches(resolved_path))
    {
        return Err(TrustError::UntrustedPath {
            program,
            path: path_display,
        });
    }

    match &policy.publisher_policy {
        PublisherPolicy::NotApplicable => Ok(VerifiedExecutable {
            program: policy.program,
            path: resolved_path.to_path_buf(),
            signer: None,
        }),
        PublisherPolicy::Required { allowed_publishers } => {
            let facts = require_trusted_chain(signer, program, &path_display)?;
            if !allowed_publishers
                .iter()
                .any(|publisher| publisher_matches(publisher, facts))
            {
                return Err(TrustError::UntrustedPublisher {
                    program,
                    path: path_display,
                });
            }
            Ok(VerifiedExecutable {
                program: policy.program,
                path: resolved_path.to_path_buf(),
                signer: Some(facts.clone()),
            })
        }
        PublisherPolicy::OptionalForKnownSystemBinary { expected_file } => {
            // The resolved file must be the named system binary...
            let file_name_ok = resolved_path
                .file_name()
                .map(|name| {
                    name.to_string_lossy()
                        .eq_ignore_ascii_case(expected_file.file_name)
                })
                .unwrap_or(false);
            if !file_name_ok {
                return Err(TrustError::UntrustedPath {
                    program,
                    path: path_display,
                });
            }
            // ...and still carry a trusted chain from the expected publisher.
            let facts = require_trusted_chain(signer, program, &path_display)?;
            if !publisher_matches(&expected_file.expected_publisher, facts) {
                return Err(TrustError::UntrustedPublisher {
                    program,
                    path: path_display,
                });
            }
            Ok(VerifiedExecutable {
                program: policy.program,
                path: resolved_path.to_path_buf(),
                signer: Some(facts.clone()),
            })
        }
    }
}

fn require_trusted_chain<'a>(
    signer: Option<&'a SignerFacts>,
    program: &'static str,
    path_display: &str,
) -> Result<&'a SignerFacts, TrustError> {
    let facts = signer.ok_or_else(|| TrustError::ExecutableIdentityMismatch {
        program,
        path: path_display.to_owned(),
    })?;
    if !facts.trusted_chain {
        return Err(TrustError::ExecutableIdentityMismatch {
            program,
            path: path_display.to_owned(),
        });
    }
    Ok(facts)
}

/// Publisher match: pinned thumbprint wins; otherwise exact case-insensitive
/// subject-CN equality. Never a substring test on a display name.
fn publisher_matches(expected: &PublisherIdentity, facts: &SignerFacts) -> bool {
    if let Some(pinned) = expected.thumbprint_sha1 {
        return pinned.eq_ignore_ascii_case(&facts.thumbprint_sha1);
    }
    normalize_cn(expected.subject_cn) == normalize_cn(&facts.subject_cn)
}

fn normalize_cn(cn: &str) -> String {
    cn.trim().to_ascii_lowercase()
}

fn path_eq_ignore_case(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .eq_ignore_ascii_case(&b.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(dir: &str, file: &'static str) -> TrustedPathRule {
        TrustedPathRule {
            label: "test",
            directory: PathBuf::from(dir),
            file_name: file,
        }
    }

    fn microsoft() -> PublisherIdentity {
        PublisherIdentity {
            label: "Microsoft",
            subject_cn: "Microsoft Corporation",
            thumbprint_sha1: None,
        }
    }

    fn required_policy() -> TrustedProgramPolicy {
        TrustedProgramPolicy {
            program: TrustedProgram::Winget,
            allowed_paths: vec![rule(r"C:\Program Files\WindowsApps\app", "winget.exe")],
            publisher_policy: PublisherPolicy::Required {
                allowed_publishers: vec![microsoft()],
            },
        }
    }

    fn signer(chain: bool, cn: &str, thumb: &str) -> SignerFacts {
        SignerFacts {
            trusted_chain: chain,
            subject_cn: cn.to_owned(),
            thumbprint_sha1: thumb.to_owned(),
        }
    }

    #[test]
    fn approved_path_and_publisher_succeed() {
        let policy = required_policy();
        let path = PathBuf::from(r"C:\Program Files\WindowsApps\app\winget.exe");
        let facts = signer(true, "Microsoft Corporation", "AABBCC");
        let result = evaluate_trust(&policy, &path, Some(&facts));
        assert!(matches!(result, Ok(verified) if verified.program == TrustedProgram::Winget));
    }

    #[test]
    fn path_shadow_is_rejected() {
        // Same file name, wrong (attacker-writable) directory.
        let policy = required_policy();
        let path = PathBuf::from(r"C:\Users\me\Downloads\winget.exe");
        let facts = signer(true, "Microsoft Corporation", "AABBCC");
        assert!(matches!(
            evaluate_trust(&policy, &path, Some(&facts)),
            Err(TrustError::UntrustedPath { .. })
        ));
    }

    #[test]
    fn wrong_publisher_at_trusted_path_is_rejected() {
        let policy = required_policy();
        let path = PathBuf::from(r"C:\Program Files\WindowsApps\app\winget.exe");
        let facts = signer(true, "Definitely Not Microsoft", "AABBCC");
        assert!(matches!(
            evaluate_trust(&policy, &path, Some(&facts)),
            Err(TrustError::UntrustedPublisher { .. })
        ));
    }

    #[test]
    fn untrusted_chain_is_an_identity_mismatch() {
        let policy = required_policy();
        let path = PathBuf::from(r"C:\Program Files\WindowsApps\app\winget.exe");
        // Right subject name but the OS did not trust the chain (unsigned or
        // self-signed replacement forging the subject).
        let facts = signer(false, "Microsoft Corporation", "AABBCC");
        assert!(matches!(
            evaluate_trust(&policy, &path, Some(&facts)),
            Err(TrustError::ExecutableIdentityMismatch { .. })
        ));
    }

    #[test]
    fn missing_signer_is_an_identity_mismatch() {
        let policy = required_policy();
        let path = PathBuf::from(r"C:\Program Files\WindowsApps\app\winget.exe");
        assert!(matches!(
            evaluate_trust(&policy, &path, None),
            Err(TrustError::ExecutableIdentityMismatch { .. })
        ));
    }

    #[test]
    fn pinned_thumbprint_mismatch_is_rejected_even_with_right_subject() {
        let policy = TrustedProgramPolicy {
            program: TrustedProgram::Winget,
            allowed_paths: vec![rule(r"C:\Program Files\WindowsApps\app", "winget.exe")],
            publisher_policy: PublisherPolicy::Required {
                allowed_publishers: vec![PublisherIdentity {
                    label: "Microsoft (pinned)",
                    subject_cn: "Microsoft Corporation",
                    thumbprint_sha1: Some("DEADBEEF"),
                }],
            },
        };
        let path = PathBuf::from(r"C:\Program Files\WindowsApps\app\winget.exe");
        let facts = signer(true, "Microsoft Corporation", "AABBCC");
        assert!(matches!(
            evaluate_trust(&policy, &path, Some(&facts)),
            Err(TrustError::UntrustedPublisher { .. })
        ));
    }

    #[test]
    fn pinned_thumbprint_match_succeeds_case_insensitively() {
        let policy = TrustedProgramPolicy {
            program: TrustedProgram::Winget,
            allowed_paths: vec![rule(r"C:\Program Files\WindowsApps\app", "winget.exe")],
            publisher_policy: PublisherPolicy::Required {
                allowed_publishers: vec![PublisherIdentity {
                    label: "Microsoft (pinned)",
                    subject_cn: "ignored when thumbprint pinned",
                    thumbprint_sha1: Some("aabbcc"),
                }],
            },
        };
        let path = PathBuf::from(r"C:\Program Files\WindowsApps\app\winget.exe");
        let facts = signer(true, "Microsoft Corporation", "AABBCC");
        assert!(evaluate_trust(&policy, &path, Some(&facts)).is_ok());
    }

    #[test]
    fn trusted_path_casing_does_not_defeat_the_rule() {
        let policy = required_policy();
        let path = PathBuf::from(r"c:\program files\windowsapps\app\WinGet.exe");
        let facts = signer(true, "Microsoft Corporation", "AABBCC");
        assert!(evaluate_trust(&policy, &path, Some(&facts)).is_ok());
    }

    #[test]
    fn known_system_binary_requires_matching_file_name() {
        let policy = TrustedProgramPolicy {
            program: TrustedProgram::PowerShell,
            allowed_paths: vec![rule(
                r"C:\Windows\System32\WindowsPowerShell\v1.0",
                "powershell.exe",
            )],
            publisher_policy: PublisherPolicy::OptionalForKnownSystemBinary {
                expected_file: SystemBinaryIdentity {
                    file_name: "powershell.exe",
                    expected_publisher: microsoft(),
                },
            },
        };
        let path = PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
        let facts = signer(true, "Microsoft Corporation", "AABBCC");
        assert!(evaluate_trust(&policy, &path, Some(&facts)).is_ok());
    }
}
