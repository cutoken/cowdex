use anyhow::Result;
use codex_features::Feature;
use codex_protocol::config_types::ServiceTier;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ThreadSettingsOverrides;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse_completed;
use core_test_support::responses::start_mock_server;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use test_case::test_case;

#[test_case(None, false, 475_000; "default")]
#[test_case(Some(128_000), false, 121_600; "explicit_window")]
#[test_case(None, true, 828_400; "maximum_window")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sol_main_context_window_survives_resume(
    configured_window: Option<i64>,
    use_maximum: bool,
    expected_window: i64,
) -> Result<()> {
    let server = start_mock_server().await;
    mount_sse_sequence(
        &server,
        vec![sse_completed("initial"), sse_completed("resumed")],
    )
    .await;
    let build = || {
        test_codex()
            .with_model("gpt-6.1-sol")
            .with_config(move |config| {
                config.model_context_window = configured_window;
                config.use_max_context_window = use_maximum;
            })
    };
    let test = build().build_with_auto_env(&server).await?;
    test.submit_text_turn("initial turn").await?;
    assert_eq!(
        test.codex
            .token_usage_info()
            .await
            .unwrap()
            .model_context_window,
        Some(expected_window)
    );
    let resumed = build().restart(&server, &test).await?;
    resumed.submit_text_turn("resumed turn").await?;
    assert_eq!(
        resumed
            .codex
            .token_usage_info()
            .await
            .unwrap()
            .model_context_window,
        Some(expected_window)
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fast_selection_is_session_scoped_and_cold_resume_starts_standard() -> Result<()> {
    let server = start_mock_server().await;
    let responses = mount_sse_sequence(
        &server,
        (0..6)
            .map(|i| sse_completed(&format!("resp-{i}")))
            .collect(),
    )
    .await;
    let mut builder = test_codex().with_model("gpt-6.1-sol");
    let test = builder.build_with_auto_env(&server).await?;
    test.submit_text_turn("standard").await?;
    test.submit_turn_with_service_tier("fast", Some(ServiceTier::Fast.request_value()))
        .await?;
    test.submit_text_turn("still fast").await?;

    let mut resume_builder = test_codex().with_model("gpt-6.1-sol");
    let resumed = resume_builder.restart(&server, &test).await?;
    resumed.submit_text_turn("resumed standard").await?;
    resumed
        .submit_turn_with_service_tier("enable fast again", Some(ServiceTier::Fast.request_value()))
        .await?;

    let mut fresh_builder = test_codex().with_model("gpt-6.1-sol");
    let fresh = fresh_builder.build_with_auto_env(&server).await?;
    fresh.submit_text_turn("separate standard session").await?;

    let tiers: Vec<_> = responses
        .requests()
        .iter()
        .map(|request| request.body_json().get("service_tier").cloned())
        .collect();
    assert_eq!(
        tiers,
        vec![
            None,
            Some("priority".into()),
            Some("priority".into()),
            None,
            Some("priority".into()),
            None
        ]
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_limit_updates_live_session_config_and_model_guidance() -> Result<()> {
    let server = start_mock_server().await;
    let responses = mount_sse_sequence(
        &server,
        vec![sse_completed("resp-1"), sse_completed("resp-2")],
    )
    .await;
    let mut builder = test_codex()
        .with_model("gpt-6.1-sol")
        .with_config(|config| {
            config
                .features
                .enable(Feature::MultiAgentV2)
                .expect("enable v2");
        });
    let test = builder.build_with_auto_env(&server).await?;
    for limit in [4, 1] {
        test.codex
            .submit(Op::ThreadSettings {
                thread_settings: ThreadSettingsOverrides {
                    agent_limit: Some(limit),
                    ..Default::default()
                },
            })
            .await?;
        wait_for_event(&test.codex, |event| {
            matches!(event, EventMsg::ThreadSettingsApplied(_))
        })
        .await;
        assert_eq!(
            test.codex
                .config()
                .await
                .multi_agent_v2
                .max_concurrent_threads_per_session,
            limit
        );
        test.submit_text_turn("continue").await?;
    }
    let requests = responses.requests();
    assert!(
        requests[0]
            .message_input_texts("developer")
            .iter()
            .any(|text| text.contains("There are 4 available concurrency slots"))
    );
    assert!(
        requests[1]
            .message_input_texts("developer")
            .iter()
            .any(|text| text.contains("There are 1 available concurrency slots"))
    );
    Ok(())
}
