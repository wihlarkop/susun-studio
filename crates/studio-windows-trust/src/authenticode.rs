//! Safe orchestration of file-level Authenticode verification (Windows only).
//!
//! This is the safe boundary over [`crate::ffi`]. It runs the two independent
//! FFI checks — chain trust and signer extraction — and assembles a
//! [`VerifiedExecutableIdentity`]. It deliberately does *not* decide whether the
//! signer is *allowed*: that product-policy decision belongs to the daemon.
//! Whether the signature is trusted and who signed it stay separate concerns.

use std::path::Path;

use crate::error::WindowsTrustError;
use crate::ffi::{self, VerifyOutcome};
use crate::model::{
    CertificateThumbprint, PublisherIdentity, SignatureStatus, VerifiedExecutableIdentity,
    VerifiedFileIdentity,
};

/// Verifies a file's Authenticode signature — embedded or catalog — and returns
/// its signer identity. Returns a typed error, never a path-only downgrade, when
/// the file is unreadable, untrusted, or its signer cannot be read. Chain trust
/// and signer identity are decided independently in the FFI; whether the signer
/// is *allowed* is the caller's policy decision.
pub fn verify_authenticode_executable(
    path: &Path,
) -> Result<VerifiedExecutableIdentity, WindowsTrustError> {
    let metadata = std::fs::metadata(path)
        .map_err(|_| WindowsTrustError::FileUnreadable(path.display().to_string()))?;

    let signer = match ffi::verify_executable(path) {
        VerifyOutcome::Trusted(signer) => signer,
        VerifyOutcome::Untrusted => {
            return Err(WindowsTrustError::SignatureUntrusted(
                path.display().to_string(),
            ));
        }
        VerifyOutcome::SignerUnreadable => {
            return Err(WindowsTrustError::SignerUnreadable(
                path.display().to_string(),
            ));
        }
    };

    Ok(VerifiedExecutableIdentity {
        publisher: PublisherIdentity {
            subject_common_name: signer.subject_common_name,
            organization: signer.organization,
        },
        certificate_thumbprint: CertificateThumbprint::from_raw(&signer.thumbprint_hex),
        signature_status: SignatureStatus::Trusted,
        file_identity: VerifiedFileIdentity {
            path: path.to_path_buf(),
            size_bytes: metadata.len(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// System PowerShell is present on every Windows install and is Microsoft-
    /// signed — the stable, always-available positive fixture.
    #[test]
    fn signed_system_powershell_verifies_as_microsoft() {
        let path =
            std::path::PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
        if !path.exists() {
            eprintln!("skipped: {} not present", path.display());
            return;
        }
        let result = verify_authenticode_executable(&path);
        assert!(result.is_ok(), "expected Ok, got {result:?}");
        if let Ok(identity) = result {
            assert_eq!(identity.signature_status, SignatureStatus::Trusted);
            assert_eq!(identity.publisher.subject_common_name, "Microsoft Windows");
            assert_eq!(identity.certificate_thumbprint.as_hex().len(), 40);
        }
    }

    /// An arbitrary non-PE file is not Authenticode-trusted and must be rejected
    /// with a typed error, never accepted.
    #[test]
    fn unsigned_file_is_rejected() {
        let path =
            std::env::temp_dir().join(format!("studio-trust-unsigned-{}.exe", std::process::id()));
        let _ = std::fs::write(&path, b"MZ this is not a signed executable");
        let result = verify_authenticode_executable(&path);
        let _ = std::fs::remove_file(&path);
        assert!(
            matches!(result, Err(WindowsTrustError::SignatureUntrusted(_))),
            "expected SignatureUntrusted, got {result:?}"
        );
    }

    /// A missing file is a typed FileUnreadable, not a panic or a false verify.
    #[test]
    fn missing_file_is_unreadable() {
        let path = std::env::temp_dir().join("studio-trust-does-not-exist-xyz.exe");
        let result = verify_authenticode_executable(&path);
        assert!(
            matches!(result, Err(WindowsTrustError::FileUnreadable(_))),
            "expected FileUnreadable, got {result:?}"
        );
    }

    /// Skip-if-not-installed bonus coverage for the real runtime binaries. These
    /// prove publisher extraction against Red Hat / Docker signatures when the
    /// software is present; they never fail the suite when it is not.
    #[test]
    fn installed_podman_verifies_as_red_hat() {
        let Some(local) = std::env::var_os("LOCALAPPDATA") else {
            return;
        };
        let path = std::path::Path::new(&local).join(r"Programs\Podman\podman.exe");
        if !path.exists() {
            eprintln!("skipped: Podman not installed");
            return;
        }
        let result = verify_authenticode_executable(&path);
        assert!(result.is_ok(), "expected Ok, got {result:?}");
        if let Ok(identity) = result {
            assert!(
                identity.publisher.subject_common_name.contains("Red Hat"),
                "unexpected signer: {}",
                identity.publisher.subject_common_name
            );
        }
    }

    #[test]
    fn installed_docker_verifies_as_docker_inc() {
        let path =
            std::path::PathBuf::from(r"C:\Program Files\Docker\Docker\resources\bin\docker.exe");
        if !path.exists() {
            eprintln!("skipped: Docker not installed");
            return;
        }
        let result = verify_authenticode_executable(&path);
        assert!(result.is_ok(), "expected Ok, got {result:?}");
        if let Ok(identity) = result {
            assert!(
                identity.publisher.subject_common_name.contains("Docker"),
                "unexpected signer: {}",
                identity.publisher.subject_common_name
            );
        }
    }
}
