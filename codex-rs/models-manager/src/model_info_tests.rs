use super::*;
use crate::ModelsManagerConfig;
use codex_protocol::config_types::Personality;
use codex_protocol::openai_models::ApprovalMessages;
use codex_protocol::openai_models::AutoReviewMessages;
use codex_protocol::openai_models::CollaborationModeMessages;
use codex_protocol::openai_models::ModelTokenBudgetConfig;
use codex_protocol::openai_models::MultiAgentMessages;
use codex_protocol::openai_models::MultiAgentModeMessages;
use codex_protocol::openai_models::MultiAgentRoleMessages;
use codex_protocol::openai_models::PermissionMessages;
use pretty_assertions::assert_eq;

#[test]
fn catalog_wait_guidance_is_relaxed_without_changing_user_overrides() {
    let original = "If the user's request requires calling tools, start with a message in the `commentary` channel. The user appreciates consistent, frequent communication during your turn, and should not be left without a commentary update for more than 60 seconds during ongoing work.\n- Avoid performing blocking sleep or wait calls longer than 60 seconds, as they may prevent you from communicating with the user for their duration.";
    let expected = "If the user's request requires calling tools, start with a message in the `commentary` channel. Share further updates when there is meaningful progress, a blocker, a decision, or a result. Do not wake or send repetitive status messages on a fixed schedule while waiting for agents or background work.\n- Prefer long, interruptible tool waits when waiting for agents or background work. Waits longer than 60 seconds are appropriate when there is no useful independent work. Use the longest suitable wait supported by each tool, including an outer code-mode wait when applicable, to avoid repeated model wakeups and token usage. Report meaningful changes when work resumes.";
    let mut model = model_info_from_slug("unknown-model");
    model
        .model_messages
        .as_mut()
        .expect("model messages")
        .instructions_template = Some(original.to_string());
    let config = ModelsManagerConfig::default();
    let normalized = with_config_overrides(model.clone(), &config);
    assert_eq!(
        normalized.get_model_instructions(config.personality),
        expected
    );
    assert_eq!(
        with_config_overrides(normalized.clone(), &config),
        normalized
    );
    let custom = ModelsManagerConfig {
        base_instructions: Some(original.to_string()),
        ..Default::default()
    };
    assert_eq!(
        with_config_overrides(model, &custom).get_model_instructions(custom.personality),
        original
    );
}

fn config_with_personality(personality: Option<Personality>) -> ModelsManagerConfig {
    ModelsManagerConfig {
        personality_enabled: true,
        personality,
        ..Default::default()
    }
}

#[test]
fn astra_remote_efforts_preserve_manual_choices_before_config_overrides() {
    use codex_protocol::openai_models::ReasoningEffortPreset;

    for slug in ["gpt-6-astra", "gpt-6-astra-wm", "global.openai.gpt-6-astra"] {
        let mut model = model_info_from_slug(slug);
        model.default_reasoning_level = Some(ReasoningEffort::High);
        model.supported_reasoning_levels = [
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::XHigh,
            ReasoningEffort::Max,
            ReasoningEffort::Ultra,
        ]
        .into_iter()
        .map(|effort| ReasoningEffortPreset {
            description: effort.to_string(),
            effort,
        })
        .collect();
        let mut expected = model.clone();
        expected.default_reasoning_level = Some(ReasoningEffort::Medium);
        expected.supported_reasoning_levels = vec![
            ReasoningEffortPreset {
                effort: ReasoningEffort::Low,
                description: "low".to_string(),
            },
            ReasoningEffortPreset {
                effort: ReasoningEffort::Medium,
                description: "medium".to_string(),
            },
            ReasoningEffortPreset {
                effort: ReasoningEffort::High,
                description: "high".to_string(),
            },
            ReasoningEffortPreset {
                effort: ReasoningEffort::XHigh,
                description: "xhigh".to_string(),
            },
            ReasoningEffortPreset {
                effort: ReasoningEffort::Max,
                description: "max".to_string(),
            },
            ReasoningEffortPreset {
                effort: ReasoningEffort::Ultra,
                description: "Medium reasoning with automatic task delegation".to_string(),
            },
        ];
        assert_eq!(
            with_config_overrides(model, &ModelsManagerConfig::default()),
            expected
        );
    }
}

#[test]
fn base_instruction_override_is_literal_and_preserves_catalog_messages() {
    let override_instructions = "override {{ personality }}";
    let mut model = model_info_from_slug("unknown-model");
    let approvals = ApprovalMessages {
        on_request: Some("user approvals".to_string()),
        on_request_auto_review: Some("auto approvals".to_string()),
        never: Some("never approvals".to_string()),
        unless_trusted: Some("unless-trusted approvals".to_string()),
    };
    let collaboration_modes = CollaborationModeMessages {
        default: Some("default instructions".to_string()),
        plan: Some("plan instructions".to_string()),
    };
    let auto_review = AutoReviewMessages {
        policy: Some("review policy".to_string()),
        policy_template: Some("review policy template".to_string()),
    };
    let permissions = PermissionMessages {
        danger_full_access: Some("danger".to_string()),
        workspace_write: Some(String::new()),
        read_only: None,
    };
    let multi_agent = MultiAgentMessages {
        role: Some(MultiAgentRoleMessages {
            root: Some("root base".to_string()),
            subagent: Some("subagent base".to_string()),
        }),
        mode: Some(MultiAgentModeMessages {
            explicit: Some("explicit mode".to_string()),
            hint_text: Some("mode hint".to_string()),
        }),
    };
    let token_budget = ModelTokenBudgetConfig {
        reminder_threshold_tokens: 128,
        reminder_message_template: "budget reminder".to_string(),
        guidance_message: "budget guidance".to_string(),
        auto_compact_fallback_prompt: "compact prompt".to_string(),
        auto_compact_fallback_buffer_tokens: 64,
    };
    model.model_messages = Some(ModelMessages {
        instructions_template: Some("template".to_string()),
        instructions_variables: Some(ModelInstructionsVariables {
            personality_default: Some("default".to_string()),
            personality_friendly: Some("friendly".to_string()),
            personality_pragmatic: Some("pragmatic".to_string()),
        }),
        approvals: Some(approvals.clone()),
        collaboration_modes: Some(collaboration_modes.clone()),
        auto_review: Some(auto_review.clone()),
        permissions: Some(permissions.clone()),
        multi_agent: Some(multi_agent.clone()),
        token_budget: Some(token_budget.clone()),
    });
    let config = ModelsManagerConfig {
        base_instructions: Some(override_instructions.to_string()),
        ..Default::default()
    };

    let updated = with_config_overrides(model, &config);

    assert_eq!(
        updated.model_messages,
        Some(ModelMessages {
            instructions_template: Some(override_instructions.to_string()),
            instructions_variables: None,
            approvals: Some(approvals),
            collaboration_modes: Some(collaboration_modes),
            auto_review: Some(auto_review),
            permissions: Some(permissions),
            multi_agent: Some(multi_agent),
            token_budget: Some(token_budget),
        })
    );
    assert_eq!(
        updated.get_model_instructions(/*personality*/ None),
        override_instructions
    );
}

#[test]
fn disabled_personality_bakes_default_and_preserves_catalog_approval_messages() {
    let mut model = model_info_from_slug("unknown-model");
    let approvals = ApprovalMessages {
        on_request: Some("user approvals".to_string()),
        on_request_auto_review: None,
        never: None,
        unless_trusted: None,
    };
    model.model_messages = Some(ModelMessages {
        instructions_template: Some("before {{ personality }} after".to_string()),
        instructions_variables: Some(ModelInstructionsVariables {
            personality_default: Some("default".to_string()),
            personality_friendly: Some("friendly".to_string()),
            personality_pragmatic: Some("pragmatic".to_string()),
        }),
        approvals: Some(approvals.clone()),
        collaboration_modes: None,
        auto_review: None,
        permissions: None,
        multi_agent: None,
        token_budget: None,
    });
    let config = ModelsManagerConfig {
        personality_enabled: false,
        ..Default::default()
    };

    let updated = with_config_overrides(model, &config);

    assert_eq!(
        updated.model_messages,
        Some(ModelMessages {
            instructions_template: Some("before default after".to_string()),
            instructions_variables: None,
            approvals: Some(approvals),
            collaboration_modes: None,
            auto_review: None,
            permissions: None,
            multi_agent: None,
            token_budget: None,
        })
    );
}

#[test]
fn disabled_personality_uses_plain_base_instructions_for_local_personality_models() {
    let config = ModelsManagerConfig {
        personality_enabled: false,
        personality: Some(Personality::Friendly),
        ..Default::default()
    };

    for slug in ["gpt-5.2-codex", "exp-codex-personality"] {
        let updated = with_config_overrides(model_info_from_slug(slug), &config);

        assert_eq!(
            updated.model_messages,
            Some(ModelMessages {
                instructions_template: Some(BASE_INSTRUCTIONS.to_string()),
                instructions_variables: None,
                approvals: None,
                collaboration_modes: None,
                auto_review: None,
                permissions: None,
                multi_agent: None,
                token_budget: None,
            }),
            "unexpected model messages for {slug}"
        );
    }
}

#[test]
fn personality_none_strips_catalog_instruction_sources_through_the_next_h1() {
    let cases = [
        (
            "Intro\n\n# Personality\n\nRemove me\n\n## Writing Style\n\nRemove me too\n\n# Safety\n\nKeep me",
            "Intro\n\n# Safety\n\nKeep me",
        ),
        ("Intro\n\n# Personality\n\nRemove me", "Intro\n\n"),
        (
            "Intro\n\n## Personality\n\nKeep me",
            "Intro\n\n## Personality\n\nKeep me",
        ),
        (
            "Intro\n\n# Personality \n\nKeep me",
            "Intro\n\n# Personality \n\nKeep me",
        ),
        (
            "Intro\r\n\r\n# Personality\r\n\r\nRemove me\r\n\r\n## Writing Style\r\n\r\nRemove me too\r\n\r\n# General\r\n\r\nKeep me",
            "Intro\r\n\r\n# General\r\n\r\nKeep me",
        ),
    ];
    let config = config_with_personality(Some(Personality::None));

    for (instructions, expected) in cases {
        let mut model = model_info_from_slug("unknown-model");
        model.model_messages = Some(ModelMessages {
            instructions_template: Some(instructions.to_string()),
            instructions_variables: None,
            approvals: None,
            collaboration_modes: None,
            auto_review: None,
            permissions: None,
            multi_agent: None,
            token_budget: None,
        });

        let updated = with_config_overrides(model, &config);
        let instructions_template = updated
            .model_messages
            .as_ref()
            .and_then(|messages| messages.instructions_template.as_deref());

        assert_eq!(
            (
                updated.get_model_instructions(Some(Personality::None)),
                instructions_template
            ),
            (expected.to_string(), Some(expected))
        );
    }
}

#[test]
fn baked_personality_section_is_preserved_without_enabled_explicit_none() {
    let instructions = "Intro\n# Personality\nKeep me\n# General\nKeep me too";
    let configs = [
        config_with_personality(/*personality*/ None),
        config_with_personality(Some(Personality::Friendly)),
        config_with_personality(Some(Personality::Pragmatic)),
        ModelsManagerConfig {
            personality: Some(Personality::None),
            ..Default::default()
        },
    ];

    for config in configs {
        let mut model = model_info_from_slug("unknown-model");
        model
            .model_messages
            .as_mut()
            .expect("fallback model messages")
            .instructions_template = Some(instructions.to_string());

        assert_eq!(
            with_config_overrides(model, &config).get_model_instructions(config.personality),
            instructions
        );
    }
}

#[test]
fn unknown_model_uses_builtin_instruction_template() {
    let model = model_info_from_slug("unknown-model");

    assert_eq!(
        model.get_model_instructions(/*personality*/ None),
        BASE_INSTRUCTIONS
    );
    assert!(model.used_fallback_model_metadata);
}

#[test]
fn model_context_window_override_clamps_to_max_context_window() {
    let mut model = model_info_from_slug("unknown-model");
    model.context_window = Some(273_000);
    model.max_context_window = Some(400_000);
    let config = ModelsManagerConfig {
        model_context_window: Some(500_000),
        ..Default::default()
    };

    let updated = with_config_overrides(model.clone(), &config);
    let mut expected = model;
    expected.context_window = Some(400_000);

    assert_eq!(updated, expected);
}

#[test]
fn model_context_window_uses_model_value_without_override() {
    let mut model = model_info_from_slug("unknown-model");
    model.context_window = Some(273_000);
    model.max_context_window = Some(400_000);
    let config = ModelsManagerConfig::default();

    let updated = with_config_overrides(model.clone(), &config);

    assert_eq!(updated, model);
}
