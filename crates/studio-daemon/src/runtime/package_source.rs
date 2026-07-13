use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use tokio::process::Command;
use tokio::time::timeout;

use super::command::SoftwareProvenance;

const MAX_SOURCE_EXPORT_BYTES: usize = 64 * 1024;
const EXPECTED_SOURCE_TYPE: &str = "Microsoft.PreIndexed.Package";
const REQUIRED_TRUST_LEVELS: &[&str] = &["Trusted", "StoreOrigin"];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WingetSourceExport {
    arg: String,
    identifier: String,
    name: String,
    trust_level: Vec<String>,
    #[serde(rename = "Type")]
    source_type: String,
}

pub async fn verify(
    winget_path: &Path,
    provenance: &SoftwareProvenance,
) -> Result<(), &'static str> {
    let mut command = Command::new(winget_path);
    command
        .env_clear()
        .args([
            "source",
            "export",
            "--name",
            provenance.source,
            "--disable-interactivity",
        ])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = timeout(Duration::from_secs(30), command.output())
        .await
        .map_err(|_| "package source verification timed out")?
        .map_err(|_| "package source verification could not start")?;
    if !output.status.success() {
        return Err("package source verification failed");
    }
    validate_export(&output.stdout, provenance)
}

fn validate_export(json: &[u8], provenance: &SoftwareProvenance) -> Result<(), &'static str> {
    if json.len() > MAX_SOURCE_EXPORT_BYTES {
        return Err("package source metadata exceeded the size limit");
    }
    let json = json.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(json);
    let source: WingetSourceExport =
        serde_json::from_slice(json).map_err(|_| "package source metadata was invalid")?;
    let trusted = REQUIRED_TRUST_LEVELS
        .iter()
        .all(|required| source.trust_level.iter().any(|actual| actual == required));
    if source.name != provenance.source
        || source.arg != provenance.source_url
        || source.identifier != provenance.source_identifier
        || source.source_type != EXPECTED_SOURCE_TYPE
        || !trusted
    {
        return Err("package source identity did not match Studio policy");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLICY: SoftwareProvenance = SoftwareProvenance {
        package_id: "RedHat.Podman",
        source: "winget",
        source_url: "https://cdn.winget.microsoft.com/cache",
        source_identifier: "Microsoft.Winget.Source_8wekyb3d8bbwe",
        expected_publisher: "Red Hat, Inc.",
        version_intent: "Latest version published by the pinned source",
        restart_impact: "No automatic restart",
    };

    #[test]
    fn official_winget_source_is_accepted() {
        let json = br#"{
            "Arg":"https://cdn.winget.microsoft.com/cache",
            "Data":"Microsoft.Winget.Source_8wekyb3d8bbwe",
            "Explicit":false,
            "Identifier":"Microsoft.Winget.Source_8wekyb3d8bbwe",
            "Name":"winget",
            "TrustLevel":["Trusted","StoreOrigin"],
            "Type":"Microsoft.PreIndexed.Package"
        }"#;
        assert!(validate_export(json, &POLICY).is_ok());
    }

    #[test]
    fn source_name_with_spoofed_url_is_rejected() {
        let json = br#"{
            "Arg":"https://example.invalid/cache",
            "Identifier":"Microsoft.Winget.Source_8wekyb3d8bbwe",
            "Name":"winget",
            "TrustLevel":["Trusted","StoreOrigin"],
            "Type":"Microsoft.PreIndexed.Package"
        }"#;
        assert_eq!(
            validate_export(json, &POLICY),
            Err("package source identity did not match Studio policy")
        );
    }

    #[test]
    fn source_without_required_trust_levels_is_rejected() {
        let json = br#"{
            "Arg":"https://cdn.winget.microsoft.com/cache",
            "Identifier":"Microsoft.Winget.Source_8wekyb3d8bbwe",
            "Name":"winget",
            "TrustLevel":["Trusted"],
            "Type":"Microsoft.PreIndexed.Package"
        }"#;
        assert_eq!(
            validate_export(json, &POLICY),
            Err("package source identity did not match Studio policy")
        );
    }
}
