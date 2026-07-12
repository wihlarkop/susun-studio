//! Safe, platform-neutral result and policy types. Nothing here touches Win32:
//! the `#[cfg(windows)]` FFI layer produces these values, and the daemon
//! consumes them. Keeping the model host-independent lets the matching logic be
//! unit-tested anywhere and lets a future macOS/Linux backend reuse the same
//! shapes.

use std::path::PathBuf;

/// A normalized signer identity extracted from a certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublisherIdentity {
    /// The leaf certificate's subject common name, e.g. `Microsoft Windows`.
    pub subject_common_name: String,
    /// The subject organization, when present, e.g. `Microsoft Corporation`.
    pub organization: Option<String>,
}

/// A SHA-1 certificate thumbprint, stored normalized: uppercase hex, no spaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificateThumbprint(String);

impl CertificateThumbprint {
    /// Normalizes any hex spelling (mixed case, embedded spaces) into the
    /// canonical uppercase, separator-free form.
    pub fn from_raw(raw: &str) -> Self {
        Self(normalize_thumbprint(raw))
    }

    pub fn as_hex(&self) -> &str {
        &self.0
    }

    /// Exact match against another thumbprint spelling, comparing normalized
    /// forms. Never a substring test.
    pub fn matches(&self, other: &str) -> bool {
        self.0 == normalize_thumbprint(other)
    }
}

/// Whether the OS trust provider accepted the file's Authenticode signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureStatus {
    /// `WinVerifyTrust` accepted the certificate chain under the Authenticode
    /// policy.
    Trusted,
    /// A signature is absent or the chain was not trusted.
    Untrusted,
}

/// Identity of the exact file that was verified — the same file the caller must
/// launch (time-of-check/time-of-use protection is the FFI layer's job).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedFileIdentity {
    pub path: PathBuf,
    pub size_bytes: u64,
}

/// The result of file-level Authenticode verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedExecutableIdentity {
    pub publisher: PublisherIdentity,
    pub certificate_thumbprint: CertificateThumbprint,
    pub signature_status: SignatureStatus,
    pub file_identity: VerifiedFileIdentity,
}

/// One acceptable signer for a trusted program. Matching prefers a pinned
/// thumbprint; otherwise it is exact, case-insensitive subject-CN equality —
/// never a substring test on a display name.
#[derive(Debug, Clone)]
pub struct AllowedPublisher {
    pub label: &'static str,
    pub subject_common_name: &'static str,
    pub thumbprint_sha1: Option<&'static str>,
}

impl AllowedPublisher {
    /// Whether this allow-list entry accepts a verified identity. A file whose
    /// chain was not trusted is never accepted, regardless of its claimed
    /// subject name.
    pub fn accepts(&self, identity: &VerifiedExecutableIdentity) -> bool {
        if identity.signature_status != SignatureStatus::Trusted {
            return false;
        }
        if let Some(pinned) = self.thumbprint_sha1 {
            return identity.certificate_thumbprint.matches(pinned);
        }
        normalize_cn(self.subject_common_name)
            == normalize_cn(&identity.publisher.subject_common_name)
    }
}

/// Whether any entry in an allow-list accepts a verified identity.
pub fn any_publisher_accepts(
    allowed: &[AllowedPublisher],
    identity: &VerifiedExecutableIdentity,
) -> bool {
    allowed.iter().any(|publisher| publisher.accepts(identity))
}

// --- MSIX package identity -------------------------------------------------

/// A package family name, e.g. `Microsoft.DesktopAppInstaller_8wekyb3d8bbwe`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageFamilyName(pub String);

/// A package full name, e.g.
/// `Microsoft.DesktopAppInstaller_1.24.x_x64__8wekyb3d8bbwe`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageFullName(pub String);

/// A package version, kept as its dotted string form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageVersion(pub String);

/// The expected App Execution Alias registration for a package (e.g. the
/// `winget.exe` alias). `directory` is a trusted Windows app-alias location; the
/// resolved alias must sit directly inside it with `file_name`.
#[derive(Debug, Clone)]
pub struct AppExecutionAlias {
    pub directory: PathBuf,
    pub file_name: &'static str,
}

impl AppExecutionAlias {
    /// Whether `resolved` is exactly this alias (case-insensitive), rejecting an
    /// alias planted in any other directory.
    pub fn matches(&self, resolved: &std::path::Path) -> bool {
        let Some(parent) = resolved.parent() else {
            return false;
        };
        let Some(name) = resolved.file_name() else {
            return false;
        };
        parent
            .to_string_lossy()
            .eq_ignore_ascii_case(&self.directory.to_string_lossy())
            && name.to_string_lossy().eq_ignore_ascii_case(self.file_name)
    }
}

/// The trusted-program policy for a program verified through its owning MSIX
/// package (used for `winget`, whose launchable path is an App Execution Alias
/// rather than the signed implementation binary).
#[derive(Debug, Clone)]
pub struct MsixProgramPolicy {
    pub package_family_name: PackageFamilyName,
    pub allowed_publishers: Vec<AllowedPublisher>,
    pub required_alias: AppExecutionAlias,
}

/// The result of MSIX package-identity verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedMsixProgram {
    pub package_family_name: PackageFamilyName,
    pub package_full_name: PackageFullName,
    pub publisher: PublisherIdentity,
    pub version: PackageVersion,
    pub alias_path: PathBuf,
}

impl MsixProgramPolicy {
    /// Whether a package's extracted publisher matches this policy's allow-list.
    /// Package publishers have no Authenticode chain status of their own here —
    /// signature-origin trust is checked separately by the FFI layer — so this
    /// compares identity only, by exact normalized subject-CN equality.
    pub fn publisher_allowed(&self, publisher: &PublisherIdentity) -> bool {
        self.allowed_publishers.iter().any(|allowed| {
            allowed.thumbprint_sha1.is_none()
                && normalize_cn(allowed.subject_common_name)
                    == normalize_cn(&publisher.subject_common_name)
        })
    }
}

fn normalize_cn(cn: &str) -> String {
    cn.trim().to_ascii_lowercase()
}

fn normalize_thumbprint(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(status: SignatureStatus, cn: &str, thumb: &str) -> VerifiedExecutableIdentity {
        VerifiedExecutableIdentity {
            publisher: PublisherIdentity {
                subject_common_name: cn.to_owned(),
                organization: None,
            },
            certificate_thumbprint: CertificateThumbprint::from_raw(thumb),
            signature_status: status,
            file_identity: VerifiedFileIdentity {
                path: PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
                size_bytes: 454_656,
            },
        }
    }

    fn microsoft_windows() -> AllowedPublisher {
        AllowedPublisher {
            label: "Microsoft Windows",
            subject_common_name: "Microsoft Windows",
            thumbprint_sha1: None,
        }
    }

    #[test]
    fn allowed_publisher_accepts_trusted_matching_identity() {
        let id = identity(SignatureStatus::Trusted, "Microsoft Windows", "AABBCC");
        assert!(microsoft_windows().accepts(&id));
    }

    #[test]
    fn unexpected_publisher_is_rejected() {
        let id = identity(
            SignatureStatus::Trusted,
            "Definitely Not Microsoft",
            "AABBCC",
        );
        assert!(!microsoft_windows().accepts(&id));
    }

    #[test]
    fn untrusted_signature_is_rejected_even_with_right_name() {
        let id = identity(SignatureStatus::Untrusted, "Microsoft Windows", "AABBCC");
        assert!(!microsoft_windows().accepts(&id));
    }

    #[test]
    fn substring_publisher_names_do_not_match() {
        // "Microsoft" must not accept "Microsoft Windows" or vice versa.
        let allowed = AllowedPublisher {
            label: "MS",
            subject_common_name: "Microsoft",
            thumbprint_sha1: None,
        };
        let id = identity(SignatureStatus::Trusted, "Microsoft Windows", "AABBCC");
        assert!(!allowed.accepts(&id));
    }

    #[test]
    fn pinned_thumbprint_matches_exactly_and_ignores_subject() {
        let allowed = AllowedPublisher {
            label: "pinned",
            subject_common_name: "ignored when pinned",
            thumbprint_sha1: Some("aa bb cc"),
        };
        let ok = identity(SignatureStatus::Trusted, "whatever", "AABBCC");
        let wrong = identity(SignatureStatus::Trusted, "whatever", "AABBCD");
        assert!(allowed.accepts(&ok));
        assert!(!allowed.accepts(&wrong));
    }

    #[test]
    fn thumbprint_normalizes_case_and_whitespace() {
        let thumb = CertificateThumbprint::from_raw("dc 91 e5 64");
        assert_eq!(thumb.as_hex(), "DC91E564");
        assert!(thumb.matches("DC91E564"));
        assert!(thumb.matches("dc91e564"));
    }

    #[test]
    fn any_publisher_accepts_scans_the_allowlist() {
        let allowed = vec![
            AllowedPublisher {
                label: "a",
                subject_common_name: "Some Other Corp",
                thumbprint_sha1: None,
            },
            microsoft_windows(),
        ];
        let id = identity(SignatureStatus::Trusted, "Microsoft Windows", "AABBCC");
        assert!(any_publisher_accepts(&allowed, &id));
    }

    #[test]
    fn msix_policy_matches_publisher_by_exact_cn() {
        let policy = MsixProgramPolicy {
            package_family_name: PackageFamilyName(
                "Microsoft.DesktopAppInstaller_8wekyb3d8bbwe".to_owned(),
            ),
            allowed_publishers: vec![AllowedPublisher {
                label: "Microsoft",
                subject_common_name: "Microsoft Corporation",
                thumbprint_sha1: None,
            }],
            required_alias: AppExecutionAlias {
                directory: PathBuf::from(r"C:\Users\me\AppData\Local\Microsoft\WindowsApps"),
                file_name: "winget.exe",
            },
        };
        let good = PublisherIdentity {
            subject_common_name: "Microsoft Corporation".to_owned(),
            organization: Some("Microsoft Corporation".to_owned()),
        };
        let bad = PublisherIdentity {
            subject_common_name: "Contoso".to_owned(),
            organization: None,
        };
        assert!(policy.publisher_allowed(&good));
        assert!(!policy.publisher_allowed(&bad));
    }

    #[cfg(windows)]
    #[test]
    fn execution_alias_rejects_a_planted_copy_elsewhere() {
        let alias = AppExecutionAlias {
            directory: PathBuf::from(r"C:\Users\me\AppData\Local\Microsoft\WindowsApps"),
            file_name: "winget.exe",
        };
        assert!(alias.matches(std::path::Path::new(
            r"C:\Users\me\AppData\Local\Microsoft\WindowsApps\winget.exe"
        )));
        assert!(!alias.matches(std::path::Path::new(r"C:\Users\me\Downloads\winget.exe")));
    }
}
