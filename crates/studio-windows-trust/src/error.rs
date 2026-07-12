//! Typed failures for Windows trust verification. Every verification path
//! returns one of these rather than a boolean or a stringly-typed error, so the
//! daemon can classify failures precisely and never silently downgrade to
//! path-only trust.

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WindowsTrustError {
    /// The crate was called on a non-Windows build. Verification is only
    /// meaningful on Windows; callers must treat this as "cannot verify",
    /// never as "verified".
    #[error("windows trust verification is unsupported on this platform")]
    UnsupportedPlatform,

    // --- File Authenticode verification -----------------------------------
    /// The exact file that would be launched could not be opened for hashing
    /// and signature checking.
    #[error("`{0}` could not be opened for verification")]
    FileUnreadable(String),
    /// A signature was evaluated but the OS trust provider did not accept the
    /// certificate chain (this includes unsigned files).
    #[error("`{0}` is not Authenticode-trusted")]
    SignatureUntrusted(String),
    /// The signature was trusted but the signer certificate details could not
    /// be extracted.
    #[error("the signer certificate of `{0}` could not be read")]
    SignerUnreadable(String),

    // --- MSIX package identity verification -------------------------------
    #[error("the expected package is not installed for the current user")]
    PackageNotInstalled,
    #[error("the installed package identity did not match the expected policy")]
    PackageIdentityMismatch,
    #[error("the installed package publisher did not match the expected policy")]
    PackagePublisherMismatch,
    #[error("the installed package signature is not trusted")]
    PackageSignatureUntrusted,
    #[error("package registration information was unavailable")]
    PackageRegistrationUnavailable,
    #[error("the expected execution alias is missing")]
    ExecutionAliasMissing,
    #[error("the resolved execution alias did not match the expected policy")]
    ExecutionAliasMismatch,
    #[error("more than one matching package was installed")]
    MultipleMatchingPackages,

    /// A Win32/WinRT call failed unexpectedly; `detail` is a redacted summary
    /// safe for ordinary logs.
    #[error("windows trust verification failed: {detail}")]
    VerificationFailed { detail: String },
}
