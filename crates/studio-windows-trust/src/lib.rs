//! Windows executable trust verification for Susun Studio.
//!
//! This crate is the single, isolated home for the Win32/WinRT FFI that proves
//! *which* executable a runtime action will launch and *who* signed it. It
//! exposes only safe, platform-neutral result types ([`model`]) and typed
//! failures ([`error`]); no `windows`-crate types, raw pointers, handles, or
//! `HRESULT`s ever cross the crate boundary. The daemon depends on it only on
//! Windows and remains `forbid(unsafe_code)`.
//!
//! Layers land incrementally:
//! - **Now:** the safe model, typed errors, and platform-neutral matching.
//! - **Next:** the `#[cfg(windows)]` FFI — Authenticode file verification
//!   (`WinVerifyTrust` + certificate extraction) and MSIX package-identity
//!   verification (`PackageManager` + App Execution Alias) — behind the
//!   `verify_authenticode_executable` / `verify_msix_alias` entry points.
//!
//! Unsafe is denied crate-wide and re-enabled only inside the documented `ffi`
//! modules that will hold the FFI.

pub mod elevation;
pub mod error;
pub mod model;

pub use elevation::{ElevatedProcessOutcome, ElevationError, run_elevated_process};
pub use error::WindowsTrustError;
pub use model::{
    AllowedPublisher, AppExecutionAlias, CertificateThumbprint, MsixProgramPolicy,
    PackageFamilyName, PackageFullName, PackageVersion, PublisherIdentity, SignatureStatus,
    VerifiedExecutableIdentity, VerifiedFileIdentity, VerifiedMsixProgram, any_publisher_accepts,
};

#[cfg(windows)]
mod authenticode;
#[cfg(windows)]
mod ffi;
#[cfg(windows)]
mod msix;

#[cfg(windows)]
pub use authenticode::verify_authenticode_executable;
#[cfg(windows)]
pub use msix::verify_msix_alias;

/// Non-Windows stub: verification is only meaningful on Windows. Callers must
/// treat this as "cannot verify" — never as "verified".
#[cfg(not(windows))]
pub fn verify_authenticode_executable(
    _path: &std::path::Path,
) -> Result<VerifiedExecutableIdentity, WindowsTrustError> {
    Err(WindowsTrustError::UnsupportedPlatform)
}

/// Non-Windows stub for MSIX package verification. See above.
#[cfg(not(windows))]
pub fn verify_msix_alias(
    _policy: &MsixProgramPolicy,
) -> Result<VerifiedMsixProgram, WindowsTrustError> {
    Err(WindowsTrustError::UnsupportedPlatform)
}
