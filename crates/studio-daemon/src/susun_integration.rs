use std::path::PathBuf;

use susun::{ProjectSummary, SusunWorkspace, render_diagnostics_json};

pub struct AnalyzedImport {
    pub source_id: Option<String>,
    pub project_name: Option<String>,
    pub project_directory: PathBuf,
    pub summary: ProjectSummary,
    pub diagnostics: serde_json::Value,
    pub has_errors: bool,
}

pub fn analyze_project(
    files: &[PathBuf],
    env_file: Option<&PathBuf>,
    project_name: Option<&str>,
    profiles: &[String],
) -> Result<AnalyzedImport, susun::Error> {
    let mut workspace = SusunWorkspace::new().with_files(files.to_vec());
    if let Some(env_file) = env_file {
        workspace = workspace.with_env_file(env_file.clone());
    }
    if let Some(name) = project_name {
        workspace = workspace.with_project_name(name);
    }
    if !profiles.is_empty() {
        workspace = workspace.with_profiles(profiles.to_vec());
    }

    let project_directory = workspace.project_directory();
    let sdk_project = workspace.analyze()?;

    let diagnostics_json = render_diagnostics_json(
        &sdk_project.analysis().report,
        &sdk_project.analysis().source_map,
    );
    let diagnostics = serde_json::from_str(&diagnostics_json)
        .unwrap_or_else(|_| serde_json::json!({ "diagnostics": [] }));

    let has_errors = sdk_project.analysis().report.has_errors();
    let summary = sdk_project.summary();
    let source_id = sdk_project
        .identity()
        .map(|identity| format!("{}@{}", identity.name, identity.working_set));
    let project_name = summary.project_name.clone();

    Ok(AnalyzedImport {
        source_id,
        project_name,
        project_directory,
        summary,
        diagnostics,
        has_errors,
    })
}
