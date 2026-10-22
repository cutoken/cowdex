use anyhow::Context;
use anyhow::Result;
use codex_config::ConfigLayerEntry;
use codex_config::ConfigLayerSource;
use codex_config::ConfigLayerStack;
use codex_core::config::AgentRoleConfig;
use codex_core::config::Config;
use codex_features::Feature;
use codex_model_provider_info::ModelProviderInfo;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::BossModeSettings;
use codex_protocol::protocol::ThreadSettingsOverrides;
use core_test_support::responses::*;
use core_test_support::submit_thread_settings;
use core_test_support::test_codex::test_codex;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::time::Duration;

fn add_flags(config: &mut Config, flags: toml::Value) {
    let mut layers = config
        .config_layer_stack
        .all_layers_low_to_high()
        .cloned()
        .collect::<Vec<_>>();
    layers.push(ConfigLayerEntry::new(
        ConfigLayerSource::SessionFlags,
        flags,
    ));
    config.config_layer_stack = ConfigLayerStack::new(
        layers,
        config.config_layer_stack.requirements().clone(),
        config.config_layer_stack.requirements_toml().clone(),
    )
    .expect("valid test configuration");
}

async fn wait_for_worker(worker: &codex_core::CodexThread) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(/*secs*/ 10), async {
        while !matches!(worker.agent_status().await, AgentStatus::Completed(_)) {
            tokio::time::sleep(Duration::from_millis(/*millis*/ 10)).await;
        }
    })
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_flags_retarget_saved_openai_workers_on_resume() -> Result<()> {
    for (provider, model) in [("zai", "glm-5.3"), ("openrouter", "deepseek/model")] {
        let server = start_mock_server().await;
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
        let resumed = test_codex()
            .with_config(move |config| {
                config.model = Some(model.to_string());
                config.model_reasoning_effort = Some(ReasoningEffort::Medium);
                config.model_provider_id = provider.to_string();
                config
                    .model_providers
                    .insert(provider.to_string(), config.model_provider.clone());
                add_flags(config, toml::toml! { model_provider = provider }.into());
            })
            .restart(&server, &initial)
            .await?;
        assert_eq!(
            resumed.codex.config_snapshot().await.boss_mode,
            Some(BossModeSettings {
                enabled: true,
                model: format!("{provider}:{model}"),
                reasoning_effort: ReasoningEffort::Medium,
            })
        );
        let mock = mount_sse_once(&server, sse(vec![ev_completed("continued")])).await;
        resumed
            .submit_text_turn("Continue on the selected provider")
            .await?;
        assert_eq!(mock.single_request().body_json()["model"], json!(model));
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_workers_route_requests_and_survive_cold_resume() -> Result<()> {
    for (provider, model, fork, main_provider) in [
        ("zai", "glm-5.3", "none", None),
        ("zai", "glm-5.3", "all", None),
        ("openrouter", "deepseek/model:free", "none", None),
        ("openrouter", "deepseek/model:free", "all", None),
        ("zai", "glm-5.3", "none", Some("openrouter")),
        ("openrouter", "deepseek/model:free", "none", Some("zai")),
    ] {
        let lead_server = start_mock_server().await;
        let worker_server = start_mock_server().await;
        let worker_url = format!("{}/v1", worker_server.uri());
        let configure = move |config: &mut Config| {
            config
                .features
                .enable(Feature::MultiAgentV2)
                .expect("enable multi-agent tools");
            config.service_tier = Some("priority".to_string());
            let role_path = config.codex_home.join("external-worker.toml");
            std::fs::write(
                &role_path,
                "model = 'gpt-5.4'\nmodel_reasoning_effort = 'low'\n",
            )
            .expect("valid test configuration");
            config.agent_roles.insert(
                "external-worker".to_string(),
                AgentRoleConfig {
                    description: Some("Worker routing test".to_string()),
                    config_file: Some(role_path.to_path_buf()),
                    nickname_candidates: None,
                },
            );
            if let Some(main_provider) = main_provider {
                let mut info = config.model_provider.clone();
                info.requires_openai_auth = false;
                info.experimental_bearer_token = Some("lead-only-key".to_string());
                config.model_provider_id = main_provider.to_string();
                config.model_provider = info.clone();
                config
                    .model_providers
                    .insert(main_provider.to_string(), info.clone());
                let mut layer: toml::Value = toml::toml! { model_provider = main_provider }.into();
                layer
                    .as_table_mut()
                    .expect("valid test configuration")
                    .insert(
                        "model_providers".to_string(),
                        toml::Value::Table(
                            [(
                                main_provider.to_string(),
                                toml::Value::try_from(info).expect("valid test configuration"),
                            )]
                            .into_iter()
                            .collect(),
                        ),
                    );
                let worker_model = format!("{provider}:{model}");
                layer
                    .as_table_mut()
                    .expect("valid test configuration")
                    .insert(
                        "features".to_string(),
                        toml::toml! {
                            [multi_agent_v2.boss_mode]
                            enabled = true
                            model = worker_model
                            reasoning_effort = "high"
                        }
                        .into(),
                    );
                config.multi_agent_v2.boss_mode = Some(BossModeSettings {
                    enabled: true,
                    model: format!("{provider}:{model}"),
                    reasoning_effort: ReasoningEffort::High,
                });
                add_flags(config, layer);
            }
            config.model_providers.insert(
                provider.to_string(),
                ModelProviderInfo {
                    name: provider.to_string(),
                    base_url: Some(worker_url.clone()),
                    experimental_bearer_token: Some("worker-only-key".to_string()),
                    ..Default::default()
                },
            );
        };
        let initial = test_codex()
            .with_config(configure.clone())
            .build_with_auto_env(&lead_server)
            .await?;
        let lead = initial.codex.config_snapshot().await;
        let boss = BossModeSettings {
            enabled: true,
            model: format!("{provider}:{model}"),
            reasoning_effort: ReasoningEffort::High,
        };
        // Exercise settings validation and persistence, not just child config construction.
        submit_thread_settings(
            &initial.codex,
            ThreadSettingsOverrides {
                boss_mode: Some(boss.clone()),
                ..Default::default()
            },
        )
        .await?;
        let spawn =
            mount_sse_once(
                &lead_server,
                sse(vec![
            ev_response_created("spawn-response"),
            ev_function_call_with_namespace("spawn-worker", "collaboration", "spawn_agent", &json!({
                "task_name": "worker",
                "message": "inspect worker routing",
                "fork_turns": fork,
                "agent_type": if fork == "none" { Some("external-worker") } else { None },
                // Boss settings must override model/effort requested by the model.
                "model": "gpt-5.4",
                "reasoning_effort": "low"
            }).to_string()),
            ev_completed("spawn-response"),
        ]),
            )
            .await;
        let lead_done = mount_sse_once_match(
            &lead_server,
            |request: &wiremock::Request| {
                String::from_utf8_lossy(&request.body).contains("spawn-worker")
            },
            sse(vec![ev_completed("lead-done")]),
        )
        .await;
        let worker_done = mount_sse_once(
            &worker_server,
            sse(vec![
                ev_assistant_message("worker-result", "inspection complete"),
                ev_completed("worker-done"),
            ]),
        )
        .await;
        initial.submit_text_turn("Delegate this inspection").await?;
        let root_id = initial.session_configured.thread_id;
        let worker_id = initial
            .thread_manager
            .list_thread_ids()
            .await
            .into_iter()
            .find(|id| *id != root_id)
            .with_context(|| {
                format!(
                    "spawn {provider}/{fork} with lead {main_provider:?}: {:?}",
                    lead_done
                        .single_request()
                        .function_call_output("spawn-worker")
                )
            })?;
        let worker = initial
            .thread_manager
            .get_thread(worker_id)
            .await
            .context("load newly spawned worker")?;
        wait_for_worker(&worker).await?;
        let request = worker_done.single_request();
        assert_eq!(
            (
                request.body_json()["model"].clone(),
                request.body_json()["reasoning"]["effort"].clone(),
                request.header("authorization")
            ),
            (
                json!(model),
                json!("high"),
                Some("Bearer worker-only-key".to_string())
            )
        );
        assert_eq!(request.body_json()["service_tier"], json!(null));
        assert_eq!(
            spawn.single_request().body_json()["model"],
            json!(lead.model)
        );
        assert_ne!(
            lead_done.single_request().header("authorization"),
            Some("Bearer worker-only-key".to_string())
        );

        worker.shutdown_and_wait().await?;
        drop(worker);
        // The lead resumes with its own provider; the existing worker must keep its provider.
        let resumed = test_codex()
            .with_config(configure)
            .restart(&lead_server, &initial)
            .await?;
        drop(initial);
        assert!(resumed.thread_manager.get_thread(worker_id).await.is_err());
        assert_eq!(resumed.codex.config_snapshot().await.boss_mode, Some(boss));
        let followup = mount_sse_once(
            &lead_server,
            sse(vec![
                ev_function_call_with_namespace(
                    "follow-worker",
                    "collaboration",
                    "followup_task",
                    r#"{"target":"worker","message":"inspect after restart"}"#,
                ),
                ev_completed("follow-response"),
            ]),
        )
        .await;
        mount_sse_once_match(
            &lead_server,
            |request: &wiremock::Request| {
                String::from_utf8_lossy(&request.body).contains("follow-worker")
            },
            sse(vec![ev_completed("resumed-lead-done")]),
        )
        .await;
        let resumed_worker = mount_sse_once(
            &worker_server,
            sse(vec![
                ev_assistant_message("resumed-worker-result", "resumed inspection complete"),
                ev_completed("resumed-worker-done"),
            ]),
        )
        .await;
        resumed.submit_text_turn("Continue the worker").await?;
        let worker = resumed
            .thread_manager
            .get_thread(worker_id)
            .await
            .context("reload cold external worker")?;
        wait_for_worker(&worker).await?;
        let request = resumed_worker.single_request();
        assert_eq!(
            (
                request.body_json()["model"].clone(),
                request.header("authorization")
            ),
            (json!(model), Some("Bearer worker-only-key".to_string()))
        );
        assert_eq!(
            followup.single_request().body_json()["model"],
            json!(lead.model)
        );
        assert_eq!(
            resumed.codex.config_snapshot().await.model_provider_id,
            lead.model_provider_id
        );
    }
    Ok(())
}
