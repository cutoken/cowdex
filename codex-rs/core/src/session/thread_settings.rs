//! Handles persistent thread-settings updates shared by standalone settings
//! submissions and turn-input submission.

use super::session::Session;
use super::session::SessionSettingsUpdate;
use crate::config::ConstraintResult;
use codex_protocol::protocol::CodexErrorInfo;
use codex_protocol::protocol::ErrorEvent;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ThreadSettingsAppliedEvent;
use codex_protocol::protocol::ThreadSettingsOverrides;
use std::sync::Arc;

pub(super) async fn validate_boss_mode(
    session: &Session,
    boss_mode: Option<&codex_protocol::protocol::BossModeSettings>,
) -> ConstraintResult<()> {
    let Some(boss) = boss_mode else {
        return Ok(());
    };
    if !boss.enabled {
        return Ok(());
    }
    let config = session.get_config().await;
    let models = session
        .services
        .models_manager
        .list_models(
            codex_models_manager::manager::RefreshStrategy::Offline,
            config.http_client_factory(),
        )
        .await;
    let invalid = |field_name, candidate, allowed| codex_config::ConstraintError::InvalidValue {
        field_name,
        candidate,
        allowed,
        requirement_source: codex_config::RequirementSource::Unknown,
    };
    if !models.iter().any(|model| {
        model.model == boss.model
            && model.multi_agent_version
                != Some(codex_protocol::protocol::MultiAgentVersion::Disabled)
    }) {
        return Err(invalid(
            "boss_mode.model",
            boss.model.clone(),
            models
                .iter()
                .filter(|model| model.show_in_picker)
                .take(5)
                .map(|model| model.model.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    let info = session
        .services
        .models_manager
        .get_model_info(&boss.model, &config.to_models_manager_config())
        .await;
    use codex_protocol::openai_models::ReasoningEffort;
    if !matches!(
        boss.reasoning_effort,
        ReasoningEffort::Low
            | ReasoningEffort::Medium
            | ReasoningEffort::High
            | ReasoningEffort::XHigh
            | ReasoningEffort::Max
    ) || !info
        .supported_reasoning_levels
        .iter()
        .any(|level| level.effort == boss.reasoning_effort)
    {
        return Err(invalid(
            "boss_mode.reasoning_effort",
            boss.reasoning_effort.to_string(),
            info.supported_reasoning_levels
                .iter()
                .filter(|level| level.effort != ReasoningEffort::Ultra)
                .map(|level| level.effort.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    Ok(())
}

/// Applies standalone thread settings and reports invalid overrides through the
/// normal event stream.
pub(super) async fn update(
    session: &Arc<Session>,
    submission_id: String,
    overrides: ThreadSettingsOverrides,
) {
    let updates = prepare_update(session, overrides).await;
    if let Err(error) = apply_update(session, submission_id.clone(), updates).await {
        session
            .send_event_raw(Event {
                id: submission_id,
                msg: EventMsg::Error(ErrorEvent {
                    message: format!("invalid thread settings override: {error}"),
                    codex_error_info: Some(CodexErrorInfo::BadRequest),
                }),
            })
            .await;
    }
}

/// Converts protocol overrides into the internal settings update shape.
pub(super) async fn prepare_update(
    session: &Session,
    overrides: ThreadSettingsOverrides,
) -> SessionSettingsUpdate {
    let ThreadSettingsOverrides {
        agent_limit,
        boss_mode,
        environments,
        profile_workspace_roots,
        approval_policy,
        approvals_reviewer,
        sandbox_policy,
        permission_profile,
        active_permission_profile,
        windows_sandbox_level,
        model,
        effort,
        summary,
        service_tier,
        collaboration_mode,
        personality,
    } = overrides;
    let ultra_reasoning_effort = collaboration_mode.as_ref().and_then(|mode| {
        if mode.reasoning_effort() == Some(codex_protocol::openai_models::ReasoningEffort::Ultra) {
            effort
                .clone()
                .flatten()
                .filter(|effort| effort != &codex_protocol::openai_models::ReasoningEffort::Ultra)
        } else {
            None
        }
    });
    let collaboration_mode = match collaboration_mode {
        Some(collaboration_mode) => collaboration_mode,
        None => {
            let state = session.state.lock().await;
            // Model and reasoning effort live in CollaborationMode settings today, so
            // partial thread-settings updates refresh those fields on the active mode.
            state
                .session_configuration
                .collaboration_mode
                .with_updates(model, effort, /*developer_instructions*/ None)
        }
    };
    SessionSettingsUpdate {
        agent_limit,
        boss_mode,
        environments,
        profile_workspace_roots,
        approval_policy,
        approvals_reviewer,
        sandbox_policy,
        permission_profile,
        active_permission_profile,
        windows_sandbox_level,
        collaboration_mode: Some(collaboration_mode),
        ultra_reasoning_effort,
        reasoning_summary: summary,
        service_tier,
        personality,
        ..Default::default()
    }
}

/// Applies persistent settings and emits the resulting effective snapshot.
pub(super) async fn apply_update(
    session: &Session,
    submission_id: String,
    updates: SessionSettingsUpdate,
) -> ConstraintResult<()> {
    session.update_settings(updates).await?;
    emit_applied(session, submission_id).await;
    Ok(())
}

/// Emits the effective thread settings after a successful update.
pub(super) async fn emit_applied(session: &Session, submission_id: String) {
    let msg = applied_event(session).await;
    session
        .send_event_raw_without_materializing_rollout(Event {
            id: submission_id,
            msg,
        })
        .await;
}

/// Builds the effective thread-settings event used by live updates and
/// synthesized fork history.
pub(super) async fn applied_event(session: &Session) -> EventMsg {
    let snapshot = session.thread_config_snapshot().await;
    EventMsg::ThreadSettingsApplied(ThreadSettingsAppliedEvent {
        thread_settings: snapshot.into_thread_settings_snapshot(),
    })
}
