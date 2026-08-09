use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::Deserialize;

use crate::{auth::authorize, error::ApiError, logging, runtime, state::AppState};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimePolicyUpdateRequest {
    pub preferred_profile_id: Option<String>,
    pub expected_impact_fingerprint: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimePolicyPreviewRequest {
    pub preferred_profile_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeOnboardingCompleteRequest {
    pub choice: runtime::onboarding::OnboardingChoice,
}

pub async fn read_runtime_onboarding(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<runtime::onboarding::RuntimeOnboarding>, ApiError> {
    authorize(&state, &headers)?;
    Ok(Json(runtime::onboarding::read(&state.db).await?))
}

pub async fn complete_runtime_onboarding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RuntimeOnboardingCompleteRequest>,
) -> Result<Json<runtime::onboarding::RuntimeOnboarding>, ApiError> {
    authorize(&state, &headers)?;
    Ok(Json(
        runtime::onboarding::complete(&state.db, request.choice).await?,
    ))
}

pub async fn dismiss_runtime_onboarding(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<runtime::onboarding::RuntimeOnboarding>, ApiError> {
    authorize(&state, &headers)?;
    Ok(Json(runtime::onboarding::dismiss(&state.db).await?))
}

pub async fn reopen_runtime_onboarding(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<runtime::onboarding::RuntimeOnboarding>, ApiError> {
    authorize(&state, &headers)?;
    Ok(Json(runtime::onboarding::reopen(&state.db).await?))
}

pub async fn read_runtime_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<runtime::RuntimePreference>, ApiError> {
    authorize(&state, &headers)?;
    Ok(Json(runtime::policy::read_preference(&state.db).await?))
}

pub async fn preview_runtime_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RuntimePolicyPreviewRequest>,
) -> Result<Json<runtime::policy::RuntimePreferenceImpactPreview>, ApiError> {
    authorize(&state, &headers)?;
    Ok(Json(
        runtime::policy::preview_preference_change(
            &state.db,
            request.preferred_profile_id.as_deref(),
        )
        .await?,
    ))
}

pub async fn set_runtime_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RuntimePolicyUpdateRequest>,
) -> Result<Json<runtime::RuntimePreference>, ApiError> {
    authorize(&state, &headers)?;
    let preference = runtime::policy::commit_preference_change(
        &state.db,
        request.preferred_profile_id.as_deref(),
        &request.expected_impact_fingerprint,
    )
    .await?;
    match preference {
        runtime::policy::ContextChangeCommit::Updated(preference) => Ok(Json(preference)),
        runtime::policy::ContextChangeCommit::Rejected(rejection) => {
            Err(ApiError::ActionUnavailable(rejection.detail().to_owned()))
        }
    }
}

pub async fn runtime_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<runtime::RuntimeStatus>, ApiError> {
    authorize(&state, &headers)?;
    logging::info("runtime_status_requested", &[]);
    Ok(Json(runtime::status(&state.db).await?))
}

pub async fn runtime_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    logging::info("runtime_logs_requested", &[]);
    Ok(Json(
        serde_json::json!({ "lines": runtime::logs(&state.db).await? }),
    ))
}

pub async fn list_runtime_profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    let profiles = runtime::list_all_profiles(&state.db).await?;
    Ok(Json(serde_json::json!({ "profiles": profiles })))
}

pub async fn runtime_profile_compatibility(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(profile_id): Path<String>,
) -> Result<Json<runtime::RuntimeCompatibilityReport>, ApiError> {
    authorize(&state, &headers)?;
    let report = runtime::compatibility::report_for_profile(&state.db, &profile_id)
        .await?
        .ok_or(ApiError::RuntimeProfileNotFound)?;
    Ok(Json(report))
}

pub async fn runtime_profile_resources(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(profile_id): Path<String>,
) -> Result<Json<runtime::RuntimeResourceSnapshot>, ApiError> {
    authorize(&state, &headers)?;
    logging::info(
        "runtime_profile_resources_requested",
        &[("profile_id", profile_id.clone())],
    );
    match runtime::resource_snapshot(&state.db, &profile_id).await? {
        runtime::ResourceSnapshotOutcome::Found(snapshot) => Ok(Json(*snapshot)),
        runtime::ResourceSnapshotOutcome::NotFound => Err(ApiError::RuntimeProfileNotFound),
        runtime::ResourceSnapshotOutcome::ProviderUnavailable => Err(ApiError::ActionUnavailable(
            "The runtime provider is unavailable.".to_owned(),
        )),
    }
}

pub async fn prepare_runtime_resource_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(profile_id): Path<String>,
    Json(request): Json<runtime::RuntimeResourceUpdateRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    logging::warn(
        "runtime_resource_update_prepare_requested",
        &[("profile_id", profile_id.clone())],
    );
    let owner = runtime::stable_suffix(&state.auth_token);
    match runtime::prepare_resource_update(
        &state.db,
        &state.trusted_plans,
        &owner,
        &profile_id,
        request,
    )
    .await?
    {
        Ok(plan) => Ok(Json(serde_json::json!({ "plan": plan }))),
        Err(result) => Ok(Json(serde_json::json!({ "result": result }))),
    }
}

pub async fn forget_runtime_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(profile_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    logging::warn(
        "runtime_profile_forget_requested",
        &[("profile_id", profile_id.clone())],
    );
    match runtime::forget_profile(&state.db, &profile_id).await? {
        runtime::ForgetOutcome::Forgotten => Ok(Json(serde_json::json!({ "forgotten": true }))),
        runtime::ForgetOutcome::NotFound => Err(ApiError::RuntimeProfileNotFound),
        runtime::ForgetOutcome::NotExternal => Err(ApiError::ActionUnavailable(
            "Built-in runtime metadata can't be forgotten; use recovery or the dedicated teardown flow."
                .to_owned(),
        )),
        runtime::ForgetOutcome::StudioManaged => Err(ApiError::ActionUnavailable(
            "A Studio-managed built-in runtime can't be forgotten; use its teardown flow instead."
                .to_owned(),
        )),
    }
}

pub async fn adopt_runtime_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(profile_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    logging::warn(
        "runtime_profile_adopt_requested",
        &[("profile_id", profile_id.clone())],
    );
    match runtime::adopt_profile(&state.db, &profile_id).await? {
        runtime::AdoptOutcome::NotFound => Err(ApiError::RuntimeProfileNotFound),
        runtime::AdoptOutcome::NotBuiltIn => Err(ApiError::ActionUnavailable(
            "Only a built-in runtime can be adopted by Studio.".to_owned(),
        )),
        runtime::AdoptOutcome::AlreadyManaged => Err(ApiError::ActionUnavailable(
            "This built-in runtime is already Studio-managed.".to_owned(),
        )),
        runtime::AdoptOutcome::OwnershipUnproven => Err(ApiError::ActionUnavailable(
            "Studio cannot adopt a runtime it did not create. Remove the naming conflict and use Set up Susun Runtime."
                .to_owned(),
        )),
    }
}

pub async fn prepare_runtime_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((provider_id, action)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &headers)?;
    reject_trusted_plan_content(&body)?;
    logging::warn(
        "runtime_trusted_plan_prepare_requested",
        &[
            ("provider_id", provider_id.clone()),
            ("action", action.clone()),
        ],
    );
    let owner = runtime::stable_suffix(&state.auth_token);
    match runtime::prepare_trusted_action(
        &state.db,
        &state.trusted_plans,
        &owner,
        &provider_id,
        &action,
    )
    .await?
    {
        Ok(preview) => Ok(Json(serde_json::json!({ "plan": preview }))),
        Err(result) => Ok(Json(serde_json::json!({ "result": result }))),
    }
}

pub async fn execute_runtime_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(plan_id): Path<String>,
    body: Bytes,
) -> Result<Json<runtime::RuntimeActionResult>, ApiError> {
    authorize(&state, &headers)?;
    reject_trusted_plan_content(&body)?;
    logging::warn(
        "runtime_trusted_plan_execute_requested",
        &[("plan_id", plan_id.clone())],
    );
    let owner = runtime::stable_suffix(&state.auth_token);
    let result =
        runtime::execute_trusted_action(&state.db, &state.trusted_plans, &owner, &plan_id).await;
    logging::info(
        "runtime_trusted_plan_execute_completed",
        &[
            ("plan_id", plan_id),
            ("action", result.action.clone()),
            ("status", result.status.clone()),
        ],
    );
    Ok(Json(result))
}

pub async fn cancel_runtime_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(plan_id): Path<String>,
    body: Bytes,
) -> Result<Json<runtime::RuntimeActionResult>, ApiError> {
    authorize(&state, &headers)?;
    reject_trusted_plan_content(&body)?;
    logging::info(
        "runtime_trusted_plan_cancel_requested",
        &[("plan_id", plan_id.clone())],
    );
    let owner = runtime::stable_suffix(&state.auth_token);
    let result = runtime::cancel_trusted_action(&state.trusted_plans, &owner, &plan_id);
    logging::info(
        "runtime_trusted_plan_cancel_completed",
        &[("plan_id", plan_id), ("status", result.status.clone())],
    );
    Ok(Json(result))
}

fn reject_trusted_plan_content(body: &Bytes) -> Result<(), ApiError> {
    if body.is_empty() {
        Ok(())
    } else {
        Err(ApiError::TrustedPlanContentRejected)
    }
}

#[cfg(test)]
mod tests {
    use axum::Json;
    use turso::params;

    use super::*;
    use crate::test_support::{authorized_headers, fresh_db, test_state};

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    async fn insert_available_profile(state: &AppState, id: &str) -> TestResult {
        let conn = state.db.connect()?;
        conn.execute(
            "INSERT INTO runtime_profiles (
                id, provider_id, provider_runtime_key, display_name, product, platform,
                runtime_class, ownership_state, source,
                installation_state, process_state, connection_state,
                availability_state, observation_revision, observed_at_ms, created_at_ms, updated_at_ms
            ) VALUES (?1, 'windows-docker-desktop', ?2, ?3, 'docker-desktop', 'windows',
                'external_local', 'external', 'provider_discovery',
                'installed', 'running', 'summarized', 'available', 0, 1, 1, 1)",
            params![id.to_owned(), format!("engine-{id}"), format!("Runtime {id}")],
        )
        .await?;
        Ok(())
    }

    #[test]
    fn trusted_plan_endpoints_reject_frontend_executable_content() {
        let body = Bytes::from_static(
            br#"{"executable":"evil.exe","args":["& calc"],"env":{"TOKEN":"secret"},"elevation":"admin"}"#,
        );
        assert!(matches!(
            reject_trusted_plan_content(&body),
            Err(ApiError::TrustedPlanContentRejected)
        ));
        assert!(reject_trusted_plan_content(&Bytes::new()).is_ok());
    }

    #[tokio::test]
    async fn runtime_status_exposes_policy_preference_and_provider_experience() -> TestResult {
        let state = test_state(fresh_db("runtime-status-contract").await?);
        insert_available_profile(&state, "profile-status").await?;
        assert!(matches!(
            runtime::policy::set_preferred(&state.db, Some("profile-status")).await?,
            runtime::policy::SetPreferredOutcome::Updated
        ));

        let response = runtime_status(State(state), authorized_headers()).await?.0;
        let value = serde_json::to_value(response)?;

        assert_eq!(value["policy"]["binding"]["source"], "global_preference");
        let providers = value["providers"].as_array().ok_or("providers")?;
        let podman = providers
            .iter()
            .find(|provider| provider["provider_id"] == "windows-podman")
            .ok_or("podman provider")?;
        assert_eq!(podman["experience"]["can_create_builtin"], true);
        assert_eq!(podman["experience"]["can_discover_external"], true);
        assert_eq!(podman["experience"]["requires_external_desktop_app"], false);
        let docker = providers
            .iter()
            .find(|provider| provider["provider_id"] == "windows-docker-desktop")
            .ok_or("docker provider")?;
        assert_eq!(docker["experience"]["can_create_builtin"], false);
        assert_eq!(docker["experience"]["requires_external_desktop_app"], true);
        let profile = docker["profiles"]
            .as_array()
            .ok_or("docker profiles")?
            .iter()
            .find(|profile| profile["id"] == "profile-status")
            .ok_or("status profile")?;
        assert_eq!(profile["runtime_class"], "external_local");
        assert_eq!(profile["is_preferred"], true);
        Ok(())
    }

    #[tokio::test]
    async fn policy_routes_require_fresh_impact_previews_and_reject_stale_commits() -> TestResult {
        let state = test_state(fresh_db("runtime-policy-routes").await?);
        insert_available_profile(&state, "profile-ready").await?;
        let preview = preview_runtime_policy(
            State(state.clone()),
            authorized_headers(),
            Json(RuntimePolicyPreviewRequest {
                preferred_profile_id: Some("profile-ready".to_owned()),
            }),
        )
        .await?
        .0;
        assert!(preview.change_allowed);

        let response = set_runtime_policy(
            State(state.clone()),
            authorized_headers(),
            Json(RuntimePolicyUpdateRequest {
                preferred_profile_id: Some("profile-ready".to_owned()),
                expected_impact_fingerprint: preview.impact_fingerprint,
            }),
        )
        .await?
        .0;
        assert_eq!(
            response.preferred_profile_id.as_deref(),
            Some("profile-ready")
        );
        assert_eq!(response.binding.state, runtime::RuntimeBindingState::Ready);

        let read = read_runtime_policy(State(state.clone()), authorized_headers())
            .await?
            .0;
        assert_eq!(read.preferred_profile_id, response.preferred_profile_id);

        let clear_preview = preview_runtime_policy(
            State(state.clone()),
            authorized_headers(),
            Json(RuntimePolicyPreviewRequest {
                preferred_profile_id: None,
            }),
        )
        .await?
        .0;
        let cleared = set_runtime_policy(
            State(state.clone()),
            authorized_headers(),
            Json(RuntimePolicyUpdateRequest {
                preferred_profile_id: None,
                expected_impact_fingerprint: clear_preview.impact_fingerprint,
            }),
        )
        .await?
        .0;
        assert_eq!(cleared.preferred_profile_id, None);

        let conn = state.db.connect()?;
        let stale_preview = preview_runtime_policy(
            State(state.clone()),
            authorized_headers(),
            Json(RuntimePolicyPreviewRequest {
                preferred_profile_id: Some("profile-ready".to_owned()),
            }),
        )
        .await?
        .0;
        conn.execute(
            "UPDATE runtime_policy SET preferred_profile_id = 'changed' WHERE singleton = 1",
            (),
        )
        .await?;
        let stale = set_runtime_policy(
            State(state.clone()),
            authorized_headers(),
            Json(RuntimePolicyUpdateRequest {
                preferred_profile_id: Some("profile-ready".to_owned()),
                expected_impact_fingerprint: stale_preview.impact_fingerprint,
            }),
        )
        .await;
        assert!(matches!(stale, Err(ApiError::ActionUnavailable(_))));

        let missing_preview = preview_runtime_policy(
            State(state.clone()),
            authorized_headers(),
            Json(RuntimePolicyPreviewRequest {
                preferred_profile_id: Some("missing".to_owned()),
            }),
        )
        .await?
        .0;
        let unknown = set_runtime_policy(
            State(state.clone()),
            authorized_headers(),
            Json(RuntimePolicyUpdateRequest {
                preferred_profile_id: Some("missing".to_owned()),
                expected_impact_fingerprint: missing_preview.impact_fingerprint,
            }),
        )
        .await;
        assert!(matches!(unknown, Err(ApiError::ActionUnavailable(_))));

        conn.execute(
            "UPDATE runtime_profiles
             SET availability_state = 'missing', missing_since_ms = 1
             WHERE id = 'profile-ready'",
            (),
        )
        .await?;
        let unavailable_preview = preview_runtime_policy(
            State(state.clone()),
            authorized_headers(),
            Json(RuntimePolicyPreviewRequest {
                preferred_profile_id: Some("profile-ready".to_owned()),
            }),
        )
        .await?
        .0;
        let unavailable = set_runtime_policy(
            State(state),
            authorized_headers(),
            Json(RuntimePolicyUpdateRequest {
                preferred_profile_id: Some("profile-ready".to_owned()),
                expected_impact_fingerprint: unavailable_preview.impact_fingerprint,
            }),
        )
        .await;
        assert!(matches!(unavailable, Err(ApiError::ActionUnavailable(_))));
        Ok(())
    }

    #[test]
    fn runtime_onboarding_completion_request_is_strict() -> Result<(), serde_json::Error> {
        let request = serde_json::from_value::<RuntimeOnboardingCompleteRequest>(
            serde_json::json!({ "choice": "existing" }),
        )?;
        assert_eq!(
            request.choice,
            runtime::onboarding::OnboardingChoice::Existing
        );
        assert!(
            serde_json::from_value::<RuntimeOnboardingCompleteRequest>(
                serde_json::json!({ "choice": "other" }),
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<RuntimeOnboardingCompleteRequest>(
                serde_json::json!({ "choice": "existing", "endpoint": "//./pipe/secret" }),
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn runtime_policy_context_requests_reject_unknown_fields() {
        assert!(
            serde_json::from_value::<RuntimePolicyPreviewRequest>(serde_json::json!({
                "preferred_profile_id": "profile",
                "endpoint": "//./pipe/secret"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<RuntimePolicyUpdateRequest>(serde_json::json!({
                "preferred_profile_id": "profile",
                "expected_impact_fingerprint": "fingerprint",
                "argv": ["unsafe"]
            }))
            .is_err()
        );
    }

    #[tokio::test]
    async fn compatibility_route_keeps_missing_profiles_as_a_normal_not_found() -> TestResult {
        let state = test_state(fresh_db("runtime-compatibility-not-found").await?);
        let result = runtime_profile_compatibility(
            State(state),
            authorized_headers(),
            Path("missing".to_owned()),
        )
        .await;
        assert!(matches!(result, Err(ApiError::RuntimeProfileNotFound)));
        Ok(())
    }

    #[tokio::test]
    async fn onboarding_routes_are_local_redacted_and_do_not_mutate_runtime_policy() -> TestResult {
        let state = test_state(fresh_db("runtime-onboarding-routes").await?);
        assert_eq!(
            read_runtime_onboarding(State(state.clone()), authorized_headers())
                .await?
                .0
                .state,
            runtime::onboarding::OnboardingState::Pending
        );

        let dismissed = dismiss_runtime_onboarding(State(state.clone()), authorized_headers())
            .await?
            .0;
        assert_eq!(
            dismissed.state,
            runtime::onboarding::OnboardingState::Dismissed
        );
        let reopened = reopen_runtime_onboarding(State(state.clone()), authorized_headers())
            .await?
            .0;
        assert_eq!(
            reopened.state,
            runtime::onboarding::OnboardingState::Pending
        );

        let completed = complete_runtime_onboarding(
            State(state.clone()),
            authorized_headers(),
            Json(RuntimeOnboardingCompleteRequest {
                choice: runtime::onboarding::OnboardingChoice::Existing,
            }),
        )
        .await?
        .0;
        assert_eq!(
            completed.state,
            runtime::onboarding::OnboardingState::Completed
        );
        let value = serde_json::to_value(completed)?;
        for forbidden in ["endpoint", "command", "argv", "executable"] {
            assert!(
                value.get(forbidden).is_none(),
                "response exposed {forbidden}"
            );
        }

        let preference = read_runtime_policy(State(state), authorized_headers())
            .await?
            .0;
        assert_eq!(preference.preferred_profile_id, None);
        Ok(())
    }
}
