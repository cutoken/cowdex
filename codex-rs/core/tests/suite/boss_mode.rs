use anyhow::Result;
use codex_config::ConfigLayerEntry;
use codex_config::ConfigLayerSource;
use codex_config::ConfigLayerStack;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::BossModeSettings;
use codex_protocol::protocol::ThreadSettingsOverrides;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::submit_thread_settings;
use core_test_support::test_codex::test_codex;
use pretty_assertions::assert_eq;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn boss_mode_live_updates_append_guidance_and_survive_cold_resume() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let initial = test_codex().build_with_auto_env(&server).await?;
    let initial_mock = mount_sse_once(
        &server,
        sse(vec![
            ev_assistant_message("initial-message", "Ready"),
            ev_completed("initial-response"),
        ]),
    )
    .await;
    initial.submit_text_turn("Start the project").await?;
    let initial_request = initial_mock.single_request();
    let initial_model = initial.codex.config_snapshot().await.model;
    let enabled = BossModeSettings {
        enabled: true,
        model: "gpt-5.4".to_string(),
        reasoning_effort: ReasoningEffort::High,
    };

    // An idle settings update must survive restart even without another user turn.
    submit_thread_settings(
        &initial.codex,
        ThreadSettingsOverrides {
            boss_mode: Some(enabled.clone()),
            ..Default::default()
        },
    )
    .await?;
    let enabled_session = test_codex().restart(&server, &initial).await?;
    let enabled_snapshot = enabled_session.codex.config_snapshot().await;
    assert_eq!(enabled_snapshot.boss_mode, Some(enabled.clone()));
    assert_eq!(enabled_snapshot.model, initial_model);

    let enabled_mock = mount_sse_once(
        &server,
        sse(vec![
            ev_assistant_message("enabled-message", "Delegating implementation"),
            ev_completed("enabled-response"),
        ]),
    )
    .await;
    enabled_session
        .submit_text_turn("Implement the plan")
        .await?;
    let enabled_request = enabled_mock.single_request();
    assert!(
        enabled_request
            .input()
            .starts_with(&initial_request.input())
    );
    let enabled_guidance = enabled_request
        .message_input_texts("developer")
        .into_iter()
        .filter(|text| text.contains("<boss_mode>"))
        .collect::<Vec<_>>();
    assert_eq!(enabled_guidance.len(), 1);
    assert!(enabled_guidance[0].contains("Boss mode is on."));
    assert!(enabled_guidance[0].contains("model gpt-5.4 with high reasoning effort"));
    assert!(
        enabled_guidance[0]
            .contains("creative direction, product and interaction design, architecture")
    );
    assert!(enabled_guidance[0].contains(
        "Aggressively delegate substantial execution work, including exploration, implementation, validation, and log analysis"
    ));

    let disabled = BossModeSettings {
        enabled: false,
        ..enabled
    };
    submit_thread_settings(
        &enabled_session.codex,
        ThreadSettingsOverrides {
            boss_mode: Some(disabled.clone()),
            ..Default::default()
        },
    )
    .await?;
    let disabled_session = test_codex().restart(&server, &enabled_session).await?;
    assert_eq!(
        disabled_session.codex.config_snapshot().await.boss_mode,
        Some(disabled)
    );

    let disabled_mock = mount_sse_once(
        &server,
        sse(vec![
            ev_assistant_message("disabled-message", "Continuing"),
            ev_completed("disabled-response"),
        ]),
    )
    .await;
    disabled_session
        .submit_text_turn("Continue the work")
        .await?;
    let disabled_request = disabled_mock.single_request();
    assert!(
        disabled_request
            .input()
            .starts_with(&enabled_request.input())
    );
    let disabled_guidance = disabled_request
        .message_input_texts("developer")
        .into_iter()
        .filter(|text| text.contains("<boss_mode>"))
        .collect::<Vec<_>>();
    assert_eq!(disabled_guidance.len(), 2);
    assert_eq!(disabled_guidance[0], enabled_guidance[0]);
    assert!(disabled_guidance[1].contains("Boss mode is off."));
    assert!(disabled_guidance[1].contains("no longer apply"));

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_astra_worker_presets_override_resumed_boss_mode() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    for enabled_setting in [
        None,
        Some("astra_ultra_low"),
        Some("astra_boss"),
        Some("astra_boss_low"),
    ] {
        let initial = test_codex().build_with_auto_env(&server).await?;
        submit_thread_settings(
            &initial.codex,
            ThreadSettingsOverrides {
                boss_mode: Some(BossModeSettings {
                    enabled: true,
                    model: "gpt-5.4".to_string(),
                    reasoning_effort: ReasoningEffort::High,
                }),
                ..Default::default()
            },
        )
        .await?;

        let astra_boss = enabled_setting == Some("astra_boss");
        let astra_ultra_low = enabled_setting == Some("astra_ultra_low");
        let astra_boss_low = enabled_setting == Some("astra_boss_low");
        let mut resume_builder = test_codex().with_config(move |config| {
            config.multi_agent_v2.astra_boss = astra_boss;
            config.multi_agent_v2.astra_ultra_low = astra_ultra_low;
            config.multi_agent_v2.astra_boss_low = astra_boss_low;
            let mut layers = config
                .config_layer_stack
                .all_layers_low_to_high()
                .cloned()
                .collect::<Vec<_>>();
            layers.push(ConfigLayerEntry::new(
                ConfigLayerSource::SessionFlags,
                toml::toml! {
                    [features.multi_agent_v2]
                    astra_boss = astra_boss
                    astra_ultra_low = astra_ultra_low
                    astra_boss_low = astra_boss_low
                }
                .into(),
            ));
            config.config_layer_stack = ConfigLayerStack::new(
                layers,
                config.config_layer_stack.requirements().clone(),
                config.config_layer_stack.requirements_toml().clone(),
            )
            .expect("Astra preset session flags should form a valid config stack layer");
        });
        let resumed = resume_builder.restart(&server, &initial).await?;
        let resumed_mock = mount_sse_once(
            &server,
            sse(vec![
                ev_assistant_message("resumed-message", "Continuing with the launch preset"),
                ev_completed("resumed-response"),
            ]),
        )
        .await;
        resumed
            .submit_text_turn("Continue with this preset")
            .await?;

        let expected_boss_mode = (astra_boss || astra_boss_low).then(|| BossModeSettings {
            enabled: true,
            model: "gpt-5.6-sol".to_string(),
            reasoning_effort: ReasoningEffort::Medium,
        });
        assert_eq!(
            resumed.codex.config_snapshot().await.boss_mode,
            expected_boss_mode,
            "preset {enabled_setting:?} should override the saved boss selection"
        );
        let resumed_request = resumed_mock.single_request();
        let boss_guidance = resumed_request
            .message_input_texts("developer")
            .into_iter()
            .filter(|text| text.contains("<boss_mode>"))
            .collect::<Vec<_>>();
        assert!(
            boss_guidance
                .last()
                .is_some_and(|guidance| guidance.contains("Boss mode is off.")),
            "preset {enabled_setting:?} should cancel saved boss guidance"
        );
    }

    Ok(())
}
