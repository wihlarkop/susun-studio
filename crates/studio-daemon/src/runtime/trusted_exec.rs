//! Daemon-side trusted execution: policy + authorization.
//!
//! The `studio-windows-trust` crate owns *platform verification* — is a file
//! Authenticode-trusted (embedded or catalog), or is winget's MSIX package
//! trusted, and who signed it. This module owns *product policy*: which absolute
//! locations and which publishers are acceptable for each [`TrustedProgram`],
//! and the decision to allow a verified identity. These are deliberately
//! separate: a cryptographically valid signature from an unexpected publisher is
//! still rejected here.
//!
//! [`verify_trusted_program`] resolves a program to its trusted absolute path,
//! verifies its identity through the crate, and authorizes the signer against
//! this module's allow-list — returning the exact path to launch. It is called
//! immediately before every process launch (and, later, at plan-preparation
//! time) so a swapped binary is caught at the last moment.

use std::path::{Path, PathBuf};

use studio_windows_trust::{
    AllowedPublisher, AppExecutionAlias, MsixProgramPolicy, PackageFamilyName,
    any_publisher_accepts, verify_authenticode_executable, verify_msix_alias,
};

use super::command::TrustedProgram;

// Allow-listed publishers, defined in trusted code only. Matched by exact
// normalized subject CN (see the trust crate); never a substring test.
const MICROSOFT_WINDOWS: AllowedPublisher = AllowedPublisher {
    label: "Microsoft Windows",
    subject_common_name: "Microsoft Windows",
    thumbprint_sha1: None,
};
const MICROSOFT_CORPORATION: AllowedPublisher = AllowedPublisher {
    label: "Microsoft Corporation",
    subject_common_name: "Microsoft Corporation",
    thumbprint_sha1: None,
};
const RED_HAT: AllowedPublisher = AllowedPublisher {
    label: "Red Hat",
    subject_common_name: "Red Hat, Inc",
    thumbprint_sha1: None,
};

/// Where a trusted program's executable may live. A resolved path must sit
/// directly in `directory` with `file_name` — never trusted merely for being on
/// `PATH`.
pub struct TrustedPathRule {
    pub directory: PathBuf,
    pub file_name: &'static str,
}

impl TrustedPathRule {
    fn candidate(&self) -> PathBuf {
        self.directory.join(self.file_name)
    }

    fn matches(&self, resolved: &Path) -> bool {
        let parent_ok = resolved
            .parent()
            .map(|parent| {
                parent
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&self.directory.to_string_lossy())
            })
            .unwrap_or(false);
        let name_ok = resolved
            .file_name()
            .map(|name| name.to_string_lossy().eq_ignore_ascii_case(self.file_name))
            .unwrap_or(false);
        parent_ok && name_ok
    }
}

/// How a program's identity is verified: by file Authenticode (embedded or
/// catalog) against a path + publisher allow-list, or — for winget — by MSIX
/// package identity.
pub enum ExecutableIdentityPolicy {
    Authenticode {
        allowed_paths: Vec<TrustedPathRule>,
        allowed_publishers: Vec<AllowedPublisher>,
    },
    Msix(MsixProgramPolicy),
}

#[derive(Debug, thiserror::Error)]
pub enum TrustFailure {
    #[error("no trusted install location found for {0}")]
    NotFound(&'static str),
    #[error("{0} resolved to an untrusted path")]
    UntrustedPath(&'static str),
    #[error("{program} could not be verified: {detail}")]
    Verification {
        program: &'static str,
        detail: String,
    },
    #[error("{0} is signed by an unapproved publisher")]
    UntrustedPublisher(&'static str),
}

/// A verified launch target: the exact absolute path to execute. Richer audit
/// detail (signer, thumbprint) is recorded by the trusted-plan store later.
pub struct VerifiedTarget {
    pub path: PathBuf,
}

fn windows_dir() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
}

fn local_appdata() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

/// The static trust policy for a program. Paths are resolved from trusted OS
/// locations, never from `PATH`.
pub fn policy_for(program: TrustedProgram) -> ExecutableIdentityPolicy {
    match program {
        TrustedProgram::PowerShell => ExecutableIdentityPolicy::Authenticode {
            allowed_paths: vec![TrustedPathRule {
                directory: windows_dir().join(r"System32\WindowsPowerShell\v1.0"),
                file_name: "powershell.exe",
            }],
            allowed_publishers: vec![MICROSOFT_WINDOWS],
        },
        #[cfg(windows)]
        TrustedProgram::Taskkill => ExecutableIdentityPolicy::Authenticode {
            allowed_paths: vec![TrustedPathRule {
                directory: windows_dir().join("System32"),
                file_name: "taskkill.exe",
            }],
            allowed_publishers: vec![MICROSOFT_WINDOWS],
        },
        TrustedProgram::Podman => ExecutableIdentityPolicy::Authenticode {
            allowed_paths: podman_paths(),
            allowed_publishers: vec![RED_HAT],
        },
        TrustedProgram::Winget => ExecutableIdentityPolicy::Msix(winget_policy()),
    }
}

fn podman_paths() -> Vec<TrustedPathRule> {
    let mut rules = Vec::new();
    if let Some(local) = local_appdata() {
        rules.push(TrustedPathRule {
            directory: local.join(r"Programs\Podman"),
            file_name: "podman.exe",
        });
    }
    rules.push(TrustedPathRule {
        directory: PathBuf::from(r"C:\Program Files\RedHat\Podman"),
        file_name: "podman.exe",
    });
    rules
}

fn winget_policy() -> MsixProgramPolicy {
    let alias_dir = local_appdata()
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join(r"Microsoft\WindowsApps");
    MsixProgramPolicy {
        package_family_name: PackageFamilyName(
            "Microsoft.DesktopAppInstaller_8wekyb3d8bbwe".to_owned(),
        ),
        allowed_publishers: vec![MICROSOFT_CORPORATION],
        required_alias: AppExecutionAlias {
            directory: alias_dir,
            file_name: "winget.exe",
        },
    }
}

/// Resolves, verifies, and authorizes a program, returning the exact absolute
/// path to launch. Any verification or authorization failure is a typed error;
/// there is no path-only fallback.
pub fn verify_trusted_program(program: TrustedProgram) -> Result<VerifiedTarget, TrustFailure> {
    let name = program.name();
    match policy_for(program) {
        ExecutableIdentityPolicy::Authenticode {
            allowed_paths,
            allowed_publishers,
        } => {
            let path = allowed_paths
                .iter()
                .map(TrustedPathRule::candidate)
                .find(|candidate| candidate.exists())
                .ok_or(TrustFailure::NotFound(name))?;
            if !allowed_paths.iter().any(|rule| rule.matches(&path)) {
                return Err(TrustFailure::UntrustedPath(name));
            }
            let identity = verify_authenticode_executable(&path).map_err(|error| {
                TrustFailure::Verification {
                    program: name,
                    detail: error.to_string(),
                }
            })?;
            if !any_publisher_accepts(&allowed_publishers, &identity) {
                return Err(TrustFailure::UntrustedPublisher(name));
            }
            Ok(VerifiedTarget { path })
        }
        ExecutableIdentityPolicy::Msix(policy) => {
            let verified =
                verify_msix_alias(&policy).map_err(|error| TrustFailure::Verification {
                    program: name,
                    detail: error.to_string(),
                })?;
            Ok(VerifiedTarget {
                path: verified.alias_path,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_rule_matches_case_insensitively_and_rejects_other_dirs() {
        let rule = TrustedPathRule {
            directory: PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0"),
            file_name: "powershell.exe",
        };
        assert!(rule.matches(Path::new(
            r"c:\windows\system32\windowspowershell\v1.0\PowerShell.exe"
        )));
        assert!(!rule.matches(Path::new(r"C:\Users\me\Downloads\powershell.exe")));
    }

    #[test]
    fn winget_uses_msix_policy_others_use_authenticode() {
        assert!(matches!(
            policy_for(TrustedProgram::Winget),
            ExecutableIdentityPolicy::Msix(_)
        ));
        assert!(matches!(
            policy_for(TrustedProgram::PowerShell),
            ExecutableIdentityPolicy::Authenticode { .. }
        ));
        assert!(matches!(
            policy_for(TrustedProgram::Podman),
            ExecutableIdentityPolicy::Authenticode { .. }
        ));
        #[cfg(windows)]
        assert!(matches!(
            policy_for(TrustedProgram::Taskkill),
            ExecutableIdentityPolicy::Authenticode { .. }
        ));
    }
}
