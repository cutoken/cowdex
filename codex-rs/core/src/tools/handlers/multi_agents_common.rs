use crate::agent::role::apply_role_to_config;
use crate::agent::role::apply_role_to_config_for_multi_agent_v2;
use crate::config::Config;
use crate::config::DEFAULT_MULTI_AGENT_V2_DEFAULT_WAIT_TIMEOUT_MS;
use crate::config::DEFAULT_MULTI_AGENT_V2_MIN_WAIT_TIMEOUT_MS;
use crate::config::HARD_MAX_MULTI_AGENT_V2_TIMEOUT_MS;
use crate::function_tool::FunctionCallError;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::session::turn_context::TurnEnvironment;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use codex_models_manager::manager::RefreshStrategy;
use codex_protocol::AgentPath;
use codex_protocol::ThreadId;
use codex_protocol::error::CodexErr;
use codex_protocol::error::CodexErrorDetails;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::openai_models::ReasoningEffortPreset;
use codex_protocol::protocol::MultiAgentVersion;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_protocol::user_input::UserInput;
use serde::Serialize;
use serde_json::Value as JsonValue;

/// Minimum wait timeout to prevent tight polling loops from burning CPU.
pub(crate) const MIN_WAIT_TIMEOUT_MS: i64 = DEFAULT_MULTI_AGENT_V2_MIN_WAIT_TIMEOUT_MS;
pub(crate) const DEFAULT_WAIT_TIMEOUT_MS: i64 = DEFAULT_MULTI_AGENT_V2_DEFAULT_WAIT_TIMEOUT_MS;
pub(crate) const MAX_WAIT_TIMEOUT_MS: i64 = HARD_MAX_MULTI_AGENT_V2_TIMEOUT_MS;
pub(crate) const MAX_SPAWN_AGENT_MODEL_OVERRIDES: usize = 5;

fn astra_boss_reasoning_effort(effort: Option<&ReasoningEffort>) -> ReasoningEffort {
    match effort {
        Some(ReasoningEffort::High)
        | Some(ReasoningEffort::XHigh)
        | Some(ReasoningEffort::Max)
        | Some(ReasoningEffort::Ultra)
        | Some(ReasoningEffort::Custom(_)) => ReasoningEffort::High,
        Some(ReasoningEffort::None)
        | Some(ReasoningEffort::Minimal)
        | Some(ReasoningEffort::Low)
        | Some(ReasoningEffort::Medium)
        | None => ReasoningEffort::Medium,
    }
}

pub(crate) fn model_supports_multi_agent_backend(
    model: &ModelPreset,
    multi_agent_version: MultiAgentVersion,
) -> bool {
    multi_agent_version != MultiAgentVersion::V2
        || model.multi_agent_version != Some(MultiAgentVersion::Disabled)
}

pub(crate) fn function_arguments(payload: ToolPayload) -> Result<String, FunctionCallError> {
    match payload {
        ToolPayload::Function { arguments } => Ok(arguments),
        _ => Err(FunctionCallError::RespondToModel(
            "collab handler received unsupported payload".to_string(),
        )),
    }
}

pub(crate) fn tool_output_json_text<T>(value: &T, tool_name: &str) -> String
where
    T: Serialize,
{
    serde_json::to_string(value).unwrap_or_else(|err| {
        JsonValue::String(format!("failed to serialize {tool_name} result: {err}")).to_string()
    })
}

pub(crate) fn tool_output_response_item<T>(
    call_id: &str,
    payload: &ToolPayload,
    value: &T,
    success: Option<bool>,
    tool_name: &str,
) -> ResponseInputItem
where
    T: Serialize,
{
    FunctionToolOutput::from_text(tool_output_json_text(value, tool_name), success)
        .to_response_item(call_id, payload)
}

pub(crate) fn tool_output_code_mode_result<T>(value: &T, tool_name: &str) -> JsonValue
where
    T: Serialize,
{
    serde_json::to_value(value).unwrap_or_else(|err| {
        JsonValue::String(format!("failed to serialize {tool_name} result: {err}"))
    })
}

pub(crate) fn collab_spawn_error(err: CodexErr) -> FunctionCallError {
    match err.details() {
        CodexErrorDetails::UnsupportedOperation(message) if message == "thread manager dropped" => {
            FunctionCallError::RespondToModel("collab manager unavailable".to_string())
        }
        CodexErrorDetails::UnsupportedOperation(message) => {
            FunctionCallError::RespondToModel(message.clone())
        }
        _ => FunctionCallError::RespondToModel(format!("collab spawn failed: {err}")),
    }
}

pub(crate) fn collab_agent_error(agent_id: ThreadId, err: CodexErr) -> FunctionCallError {
    match err.details() {
        CodexErrorDetails::ThreadNotFound(id) => {
            FunctionCallError::RespondToModel(format!("agent with id {id} not found"))
        }
        CodexErrorDetails::InternalAgentDied => {
            FunctionCallError::RespondToModel(format!("agent with id {agent_id} is closed"))
        }
        CodexErrorDetails::UnsupportedOperation(_) => {
            FunctionCallError::RespondToModel("collab manager unavailable".to_string())
        }
        _ => FunctionCallError::RespondToModel(format!("collab tool failed: {err}")),
    }
}

pub(crate) fn thread_spawn_source(
    parent_thread_id: ThreadId,
    parent_session_source: &SessionSource,
    depth: i32,
    agent_role: Option<&str>,
    task_name: Option<String>,
) -> Result<SessionSource, FunctionCallError> {
    let agent_path = task_name
        .as_deref()
        .map(|task_name| {
            parent_session_source
                .get_agent_path()
                .unwrap_or_else(AgentPath::root)
                .join(task_name)
                .map_err(FunctionCallError::RespondToModel)
        })
        .transpose()?;
    Ok(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id,
        depth,
        agent_path,
        agent_nickname: None,
        agent_role: agent_role.map(str::to_string),
    }))
}

pub(crate) fn parse_collab_input(
    message: Option<String>,
    items: Option<Vec<UserInput>>,
) -> Result<Vec<UserInput>, FunctionCallError> {
    match (message, items) {
        (Some(_), Some(_)) => Err(FunctionCallError::RespondToModel(
            "Provide either message or items, but not both".to_string(),
        )),
        (None, None) => Err(FunctionCallError::RespondToModel(
            "Provide one of: message or items".to_string(),
        )),
        (Some(message), None) => {
            if message.trim().is_empty() {
                return Err(FunctionCallError::RespondToModel(
                    "Empty message can't be sent to an agent".to_string(),
                ));
            }
            Ok(vec![UserInput::Text {
                text: message,
                text_elements: Vec::new(),
            }])
        }
        (None, Some(items)) => {
            if items.is_empty() {
                return Err(FunctionCallError::RespondToModel(
                    "Items can't be empty".to_string(),
                ));
            }
            Ok(items)
        }
    }
}

/// Builds the base config snapshot for a newly spawned sub-agent.
///
/// The returned config starts from the parent's effective config and then refreshes the
/// runtime-owned fields carried by the turn and selected environment, including model selection,
/// reasoning settings, approval policy, sandbox, and cwd. Role-specific overrides are layered
/// after this step; skipping this helper and cloning stale config state directly can send the child
/// agent out with the wrong provider or runtime policy.
pub(crate) fn build_agent_spawn_config(
    base_instructions: &BaseInstructions,
    turn: &TurnContext,
    environment: Option<&TurnEnvironment>,
) -> Result<Config, FunctionCallError> {
    let mut config = build_agent_shared_config(turn, environment)?;
    config.base_instructions = Some(base_instructions.text.clone());
    config.base_instructions_provenance = base_instructions.provenance.clone();
    Ok(config)
}

pub(crate) fn build_agent_resume_config(
    turn: &TurnContext,
    environment: Option<&TurnEnvironment>,
) -> Result<Config, FunctionCallError> {
    let mut config = build_agent_shared_config(turn, environment)?;
    // For resume, keep base instructions sourced from rollout/session metadata.
    config.base_instructions = None;
    config.base_instructions_provenance = None;
    Ok(config)
}

fn build_agent_shared_config(
    turn: &TurnContext,
    environment: Option<&TurnEnvironment>,
) -> Result<Config, FunctionCallError> {
    let base_config = turn.config.clone();
    let mut config = (*base_config).clone();
    config.model = Some(turn.model_info.slug.clone());
    config.model_provider = turn.provider.info().clone();
    config.model_reasoning_effort = turn
        .reasoning_effort
        .clone()
        .or_else(|| turn.model_info.default_reasoning_level.clone());
    config.model_reasoning_summary = Some(turn.reasoning_summary);
    config.developer_instructions = turn.developer_instructions.clone();
    if turn.multi_agent_version == MultiAgentVersion::V2
        && let Some(developer_instructions) = turn
            .config
            .multi_agent_v2
            .subagent_developer_instructions
            .clone()
    {
        config.developer_instructions = Some(developer_instructions);
    }
    apply_spawn_agent_runtime_overrides(
        &mut config,
        turn,
        &turn.config.multi_agent_v2,
        environment,
        /*astra_boss_requested_effort*/ None,
    )?;

    Ok(config)
}

pub(crate) fn reject_full_fork_agent_type_override(
    agent_type: Option<&str>,
) -> Result<(), FunctionCallError> {
    if agent_type.is_some() {
        return Err(FunctionCallError::RespondToModel(
            "Full-history forked agents inherit the parent agent type; omit agent_type, or spawn without a full-history fork.".to_string(),
        ));
    }
    Ok(())
}

/// Copies runtime-only turn state onto a child config before it is handed to `AgentControl`.
///
/// These values are chosen by the live turn and selected environment rather than persisted config,
/// so leaving them stale can make a child agent disagree with its parent about approval policy,
/// cwd, or sandboxing.
pub(crate) fn apply_spawn_agent_runtime_overrides(
    config: &mut Config,
    turn: &TurnContext,
    worker_policy: &crate::config::MultiAgentV2Config,
    environment: Option<&TurnEnvironment>,
    astra_boss_requested_effort: Option<&ReasoningEffort>,
) -> Result<(), FunctionCallError> {
    config
        .permissions
        .approval_policy
        .set(turn.approval_policy())
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("approval_policy is invalid: {err}"))
        })?;
    config.approvals_reviewer = turn.config.approvals_reviewer;
    #[allow(deprecated)]
    let turn_cwd = turn.cwd.clone();
    config.cwd = turn_cwd;
    let permission_profile = environment
        .map(|environment| environment.permission_profile().clone())
        .unwrap_or_else(|| turn.permission_profile());
    config
        .permissions
        .set_permission_profile(permission_profile)
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("permission_profile is invalid: {err}"))
        })?;
    config.multi_agent_v2 = worker_policy.clone();
    if crate::session::session_flags_select_model_provider(&turn.config) {
        config.model = Some(turn.model_info.slug.clone());
        config.model_provider_id = turn.config.model_provider_id.clone();
        config.model_provider = turn.provider.info().clone();
    } else if let Some(boss) = worker_policy.boss_mode.as_ref().filter(|boss| boss.enabled) {
        config.model = Some(boss.model.clone());
        config.model_reasoning_effort = Some(boss.reasoning_effort.clone());
    } else if worker_policy.astra_ultra_low {
        config.model = Some("gpt-6-astra".to_string());
        config.model_reasoning_effort = Some(ReasoningEffort::Low);
    } else if worker_policy.boss_mode.is_none() && worker_policy.astra_boss_low {
        config.model = Some("gpt-5.6-sol".to_string());
        config.model_reasoning_effort = Some(ReasoningEffort::Medium);
    } else if worker_policy.boss_mode.is_none() && worker_policy.astra_boss {
        config.model = Some("gpt-5.6-sol".to_string());
        config.model_reasoning_effort = Some(astra_boss_reasoning_effort(
            astra_boss_requested_effort.or(config.model_reasoning_effort.as_ref()),
        ));
    }
    Ok(())
}

pub(crate) async fn apply_requested_spawn_agent_model_overrides(
    session: &Session,
    turn: &TurnContext,
    worker_policy: &crate::config::MultiAgentV2Config,
    config: &mut Config,
    requested_model: Option<&str>,
    requested_reasoning_effort: Option<ReasoningEffort>,
) -> Result<(), FunctionCallError> {
    if crate::session::session_flags_select_model_provider(&turn.config) {
        config.model = Some(turn.model_info.slug.clone());
        config.model_provider_id = turn.config.model_provider_id.clone();
        config.model_provider = turn.provider.info().clone();
        if let Some(reasoning_effort) = requested_reasoning_effort
            .or_else(|| turn.config.agent_default_subagent_reasoning_effort.clone())
        {
            if !turn.model_info.supported_reasoning_levels.is_empty() {
                validate_spawn_agent_reasoning_effort(
                    &turn.model_info.slug,
                    &turn.model_info.supported_reasoning_levels,
                    &reasoning_effort,
                    /*boss*/ None,
                )?;
            }
            config.model_reasoning_effort = Some(reasoning_effort);
        }
        return Ok(());
    }

    let boss = worker_policy.boss_mode.as_ref().filter(|boss| boss.enabled);
    let requested_model = if let Some(boss) = boss {
        Some(boss.model.as_str())
    } else if worker_policy.astra_ultra_low {
        Some("gpt-6-astra")
    } else if worker_policy.boss_mode.is_none()
        && (worker_policy.astra_boss_low || worker_policy.astra_boss)
    {
        Some("gpt-5.6-sol")
    } else {
        requested_model.or(turn.config.agent_default_subagent_model.as_deref())
    };
    let requested_reasoning_effort = if let Some(boss) = boss {
        Some(boss.reasoning_effort.clone())
    } else if worker_policy.astra_ultra_low {
        Some(ReasoningEffort::Low)
    } else if worker_policy.boss_mode.is_none() && worker_policy.astra_boss_low {
        Some(ReasoningEffort::Medium)
    } else if worker_policy.boss_mode.is_none() && worker_policy.astra_boss {
        let effort = requested_reasoning_effort
            .as_ref()
            .or(turn.config.agent_default_subagent_reasoning_effort.as_ref());
        Some(astra_boss_reasoning_effort(effort))
    } else {
        requested_reasoning_effort
            .or_else(|| turn.config.agent_default_subagent_reasoning_effort.clone())
    };
    if requested_model.is_none() && requested_reasoning_effort.is_none() {
        return Ok(());
    }

    if let Some(requested_model) = requested_model {
        let available_models = session
            .services
            .models_manager
            .list_models(RefreshStrategy::Offline, config.http_client_factory())
            .await;
        let selected_model_name = find_spawn_agent_model_name(
            &available_models,
            requested_model,
            turn.multi_agent_version,
        )?;
        let selected_model_info = session
            .services
            .models_manager
            .get_model_info(&selected_model_name, &config.to_models_manager_config())
            .await;

        config.model = Some(selected_model_name.clone());
        if let Some(reasoning_effort) = requested_reasoning_effort {
            validate_spawn_agent_reasoning_effort(
                &selected_model_name,
                &selected_model_info.supported_reasoning_levels,
                &reasoning_effort,
                boss,
            )?;
            config.model_reasoning_effort = Some(reasoning_effort);
        } else {
            config.model_reasoning_effort = selected_model_info.default_reasoning_level;
        }

        return Ok(());
    }

    if let Some(reasoning_effort) = requested_reasoning_effort {
        validate_spawn_agent_reasoning_effort(
            &turn.model_info.slug,
            &turn.model_info.supported_reasoning_levels,
            &reasoning_effort,
            boss,
        )?;
        config.model_reasoning_effort = Some(reasoning_effort);
    }

    Ok(())
}

pub(crate) async fn apply_spawn_agent_service_tier(
    session: &Session,
    config: &mut Config,
    parent_service_tier: Option<&str>,
    requested_service_tier: Option<&str>,
) -> Result<(), FunctionCallError> {
    let candidate_service_tiers = [
        config.service_tier.clone(),
        requested_service_tier.map(str::to_string),
        parent_service_tier.map(str::to_string),
    ];
    if candidate_service_tiers.iter().all(Option::is_none) {
        config.service_tier = None;
        return Ok(());
    }

    let model = config.model.clone().ok_or_else(|| {
        FunctionCallError::RespondToModel(
            "spawn_agent could not resolve the child model for service tier validation".to_string(),
        )
    })?;
    let model_info = session
        .services
        .models_manager
        .get_model_info(model.as_str(), &config.to_models_manager_config())
        .await;

    if let Some(requested_service_tier) = requested_service_tier
        && !model_info.supports_service_tier(requested_service_tier)
    {
        let supported_service_tiers = if model_info.service_tiers.is_empty() {
            "none".to_string()
        } else {
            model_info
                .service_tiers
                .iter()
                .map(|tier| tier.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        return Err(FunctionCallError::RespondToModel(format!(
            "Service tier `{requested_service_tier}` is not supported for model `{model}`. Supported service tiers: {supported_service_tiers}"
        )));
    }

    config.service_tier =
        candidate_service_tiers
            .into_iter()
            .flatten()
            .find(|candidate_service_tier| {
                model_info.supports_service_tier(candidate_service_tier.as_str())
            });
    Ok(())
}

pub(crate) async fn apply_spawn_agent_role(
    session: &Session,
    config: &mut Config,
    role_name: Option<&str>,
) -> Result<(), FunctionCallError> {
    let previous_model = config.model.clone();
    let previous_reasoning_effort = config.model_reasoning_effort.clone();
    let boss_mode = config.multi_agent_v2.boss_mode.clone();
    if session.multi_agent_version() == Some(MultiAgentVersion::V2) {
        apply_role_to_config_for_multi_agent_v2(config, role_name)
            .await
            .map_err(FunctionCallError::RespondToModel)?;
    } else {
        apply_role_to_config(config, role_name)
            .await
            .map_err(FunctionCallError::RespondToModel)?;
    }
    config.multi_agent_v2.boss_mode = boss_mode;
    if let Some(boss) = config
        .multi_agent_v2
        .boss_mode
        .as_ref()
        .filter(|boss| boss.enabled)
    {
        config.model = Some(boss.model.clone());
        config.model_reasoning_effort = Some(boss.reasoning_effort.clone());
    }
    if config.model == previous_model && config.model_reasoning_effort == previous_reasoning_effort
    {
        return Ok(());
    }

    let Some(reasoning_effort) = config.model_reasoning_effort.clone() else {
        return Ok(());
    };
    let model = config.model.clone().ok_or_else(|| {
        FunctionCallError::RespondToModel(
            "spawn_agent could not resolve the child model for reasoning effort validation"
                .to_string(),
        )
    })?;
    let model_info = session
        .services
        .models_manager
        .get_model_info(&model, &config.to_models_manager_config())
        .await;
    if model_info.used_fallback_model_metadata {
        return Ok(());
    }

    validate_spawn_agent_reasoning_effort(
        &model,
        &model_info.supported_reasoning_levels,
        &reasoning_effort,
        config
            .multi_agent_v2
            .boss_mode
            .as_ref()
            .filter(|boss| boss.enabled),
    )
}

fn find_spawn_agent_model_name(
    available_models: &[ModelPreset],
    requested_model: &str,
    multi_agent_version: MultiAgentVersion,
) -> Result<String, FunctionCallError> {
    available_models
        .iter()
        .find(|model| {
            model.model == requested_model
                && model_supports_multi_agent_backend(model, multi_agent_version)
        })
        .map(|model| model.model.clone())
        .ok_or_else(|| {
            let available = available_models
                .iter()
                .filter(|model| model.show_in_picker)
                .filter(|model| model_supports_multi_agent_backend(model, multi_agent_version))
                .take(MAX_SPAWN_AGENT_MODEL_OVERRIDES)
                .map(|model| model.model.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            FunctionCallError::RespondToModel(format!(
                "Unknown model `{requested_model}` for spawn_agent. Available models: {available}"
            ))
        })
}

fn validate_spawn_agent_reasoning_effort(
    model: &str,
    supported_reasoning_levels: &[ReasoningEffortPreset],
    requested_reasoning_effort: &ReasoningEffort,
    boss: Option<&codex_protocol::protocol::BossModeSettings>,
) -> Result<(), FunctionCallError> {
    if !(matches!(
        requested_reasoning_effort,
        ReasoningEffort::Low
            | ReasoningEffort::Medium
            | ReasoningEffort::High
            | ReasoningEffort::XHigh
    ) || boss.is_some() && requested_reasoning_effort == &ReasoningEffort::Max)
    {
        return Err(FunctionCallError::RespondToModel(format!(
            "Reasoning effort `{requested_reasoning_effort}` is not available for Cowdex subagents. Choose one of: low, medium, high, xhigh"
        )));
    }

    if supported_reasoning_levels
        .iter()
        .any(|preset| &preset.effort == requested_reasoning_effort)
    {
        return Ok(());
    }

    let supported = supported_reasoning_levels
        .iter()
        .map(|preset| preset.effort.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(FunctionCallError::RespondToModel(format!(
        "Reasoning effort `{requested_reasoning_effort}` is not supported for model `{model}`. Supported reasoning efforts: {supported}"
    )))
}
