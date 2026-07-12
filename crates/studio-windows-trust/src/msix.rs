//! Narrow winget MSIX package-identity verification (Windows only).
//!
//! `winget`'s launchable path is a 0-byte App Execution Alias; the real signed
//! binary lives in an ACL-locked store folder we cannot open. So file
//! Authenticode is the wrong primitive here. Instead we verify the *owning MSIX
//! package* through the WinRT `PackageManager`:
//!
//! 1. exactly one installed package matches the expected family name;
//! 2. its publisher matches the daemon-owned allow-list (primary gate);
//! 3. its signature origin is a Microsoft-distributed kind (Store/System) —
//!    an additional check, not a rigid cross-version requirement;
//! 4. the expected execution alias exists at its trusted location.
//!
//! This is deliberately winget-specific: it is not a generic package launcher or
//! Store discovery API. The only `unsafe` is the COM apartment init required
//! before any WinRT call; the WinRT projections themselves are safe.
#![allow(unsafe_code)]

use windows::ApplicationModel::{Package, PackageSignatureKind};
use windows::Management::Deployment::PackageManager;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::HSTRING;

use crate::error::WindowsTrustError;
use crate::model::{
    MsixProgramPolicy, PackageFamilyName, PackageFullName, PackageVersion, PublisherIdentity,
    VerifiedMsixProgram,
};

/// Verifies the installed MSIX package behind a policy's family name and
/// confirms its execution alias is present at the trusted location. Returns a
/// typed error for every failure mode; never a path-only downgrade.
pub fn verify_msix_alias(
    policy: &MsixProgramPolicy,
) -> Result<VerifiedMsixProgram, WindowsTrustError> {
    ensure_apartment();

    let manager = PackageManager::new().map_err(winrt_err)?;
    let family = HSTRING::from(policy.package_family_name.0.as_str());
    // Empty SID = the current user; the current-user query needs no special
    // package-query capability (unlike the all-users variant).
    let packages = manager
        .FindPackagesByUserSecurityIdPackageFamilyName(&HSTRING::new(), &family)
        .map_err(winrt_err)?;

    let mut found: Vec<Package> = Vec::new();
    for package in &packages {
        found.push(package);
    }
    let package = match found.as_slice() {
        [] => return Err(WindowsTrustError::PackageNotInstalled),
        [one] => one,
        _ => return Err(WindowsTrustError::MultipleMatchingPackages),
    };

    let id = package.Id().map_err(winrt_err)?;

    // Family identity (defensive — we queried by it).
    let family_name = id.FamilyName().map_err(winrt_err)?.to_string();
    if family_name != policy.package_family_name.0 {
        return Err(WindowsTrustError::PackageIdentityMismatch);
    }

    // Publisher — the primary gate. Compared by exact normalized CN in the
    // policy; never a substring test.
    let publisher_dn = id.Publisher().map_err(winrt_err)?.to_string();
    let publisher = PublisherIdentity {
        subject_common_name: attr_from_dn(&publisher_dn, "CN")
            .unwrap_or_else(|| publisher_dn.clone()),
        organization: attr_from_dn(&publisher_dn, "O"),
    };
    if !policy.publisher_allowed(&publisher) {
        return Err(WindowsTrustError::PackagePublisherMismatch);
    }

    // Signature origin — an additional check. A Microsoft-distributed package is
    // Store- or System-signed; sideloaded/developer packages are rejected.
    let kind = package.SignatureKind().map_err(winrt_err)?;
    if kind != PackageSignatureKind::Store && kind != PackageSignatureKind::System {
        return Err(WindowsTrustError::PackageSignatureUntrusted);
    }

    // Execution alias must exist at its trusted location, and only there.
    let alias_path = policy
        .required_alias
        .directory
        .join(policy.required_alias.file_name);
    if !alias_path.exists() {
        return Err(WindowsTrustError::ExecutionAliasMissing);
    }
    if !policy.required_alias.matches(&alias_path) {
        return Err(WindowsTrustError::ExecutionAliasMismatch);
    }

    let full_name = id.FullName().map_err(winrt_err)?.to_string();
    let version = id.Version().map_err(winrt_err)?;
    let version_string = format!(
        "{}.{}.{}.{}",
        version.Major, version.Minor, version.Build, version.Revision
    );

    Ok(VerifiedMsixProgram {
        package_family_name: PackageFamilyName(family_name),
        package_full_name: PackageFullName(full_name),
        publisher,
        version: PackageVersion(version_string),
        alias_path,
    })
}

/// Ensures the calling thread is in a COM/WinRT apartment before any WinRT call.
fn ensure_apartment() {
    // SAFETY: CoInitializeEx with no reserved pointer is always safe to call.
    // Repeated calls return S_FALSE / RPC_E_CHANGED_MODE, which we ignore; the
    // thread simply stays in the (multithreaded) apartment. Deliberately not
    // paired with CoUninitialize — the daemon thread keeps its apartment.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

fn winrt_err(error: windows::core::Error) -> WindowsTrustError {
    // Only the numeric HRESULT is recorded — safe for ordinary logs.
    WindowsTrustError::VerificationFailed {
        detail: format!("0x{:08X}", error.code().0 as u32),
    }
}

/// Extracts a distinguished-name attribute value (e.g. `CN`, `O`) from a
/// publisher DN like `CN=Microsoft Corporation, O=Microsoft Corporation, ...`.
fn attr_from_dn(dn: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    dn.split(',').find_map(|part| {
        part.trim()
            .strip_prefix(&prefix)
            .map(|value| value.trim().trim_matches('"').to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AppExecutionAlias;
    use std::path::PathBuf;

    #[test]
    fn attr_from_dn_extracts_cn_and_org() {
        let dn = "CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US";
        assert_eq!(
            attr_from_dn(dn, "CN").as_deref(),
            Some("Microsoft Corporation")
        );
        assert_eq!(
            attr_from_dn(dn, "O").as_deref(),
            Some("Microsoft Corporation")
        );
        assert_eq!(attr_from_dn(dn, "X"), None);
    }

    /// Real winget verification when the Desktop App Installer is present.
    /// Skips (never fails) when winget's alias is not installed.
    #[test]
    fn installed_winget_verifies_as_microsoft() {
        let Some(local) = std::env::var_os("LOCALAPPDATA") else {
            return;
        };
        let alias_dir = PathBuf::from(&local).join(r"Microsoft\WindowsApps");
        if !alias_dir.join("winget.exe").exists() {
            eprintln!("skipped: winget alias not present");
            return;
        }
        let policy = MsixProgramPolicy {
            package_family_name: PackageFamilyName(
                "Microsoft.DesktopAppInstaller_8wekyb3d8bbwe".to_owned(),
            ),
            allowed_publishers: vec![crate::model::AllowedPublisher {
                label: "Microsoft",
                subject_common_name: "Microsoft Corporation",
                thumbprint_sha1: None,
            }],
            required_alias: AppExecutionAlias {
                directory: alias_dir,
                file_name: "winget.exe",
            },
        };
        let result = verify_msix_alias(&policy);
        assert!(result.is_ok(), "expected Ok, got {result:?}");
        if let Ok(verified) = result {
            assert_eq!(
                verified.publisher.subject_common_name,
                "Microsoft Corporation"
            );
        }
    }
}
