//! Trusted reasoning-effort updates follow surviving history and the next turn's selected settings.

use codex_features::Feature;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ThreadSettingsOverrides;
use core_test_support::responses;
use core_test_support::responses::ResponsesRequest;
use core_test_support::skip_if_no_network;
use core_test_support::submit_thread_settings;
use core_test_support::test_codex::TestCodexBuilder;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::Value;
use test_case::test_case;

fn override_builder() -> TestCodexBuilder {
    test_codex()
        .with_model_info_override("gpt-5.4", |model| {
            model.use_responses_lite = true;
        })
        .with_config(|config| {
            config
                .features
                .enable(Feature::ReasoningEffortOverride)
                .expect("enable reasoning effort overrides");
            config.model_reasoning_effort = Some(ReasoningEffort::Medium);
        })
}

fn effort_updates(request: &ResponsesRequest) -> Vec<Value> {
    request
        .input()
        .into_iter()
        .filter(|item| item["type"] == "configuration_update")
        .collect()
}

fn effort_update(effort: ReasoningEffort) -> Value {
    serde_json::json!({
        "type": "configuration_update",
        "reasoning": {"effort": effort},
    })
}

fn message(role: &str, text: &str) -> Value {
    serde_json::json!({
        "type": "message",
        "role": role,
        "content": [{"type": "input_text", "text": text}],
    })
}

#[test_case(ReasoningEffort::Low; "ultra low")]
#[test_case(ReasoningEffort::Medium; "ultra medium")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_usage_persists_actual_ultra_request_effort(
    resolved: ReasoningEffort,
) -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));
    let server = responses::start_mock_server().await;
    let mock = responses::mount_sse_once(
        &server,
        responses::sse(vec![responses::ev_completed_with_tokens("usage", 42)]),
    )
    .await;
    let configured = resolved.clone();
    let test = test_codex()
        .with_model_info_override("gpt-6-astra", |model| model.use_responses_lite = false)
        .with_config(move |config| {
            config.model_reasoning_effort = Some(ReasoningEffort::Ultra);
            config.ultra_reasoning_effort = configured;
        })
        .build_with_auto_env(&server)
        .await?;
    test.submit_text_turn("audit this request").await?;
    test.codex.flush_rollout().await?;
    let request = mock.single_request().body_json();
    assert_eq!(
        request["reasoning"]["effort"],
        serde_json::to_value(resolved)?
    );
    let rollout = std::fs::read_to_string(test.codex.rollout_path().expect("rollout path"))?;
    let completed = rollout
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter_map(|line| {
            (line["type"] == "event_msg" && line["payload"]["type"] == "raw_response_completed")
                .then(|| line["payload"].clone())
        })
        .collect::<Vec<_>>();
    assert_eq!(completed.len(), 1);
    assert_eq!(
        (
            &completed[0]["model"],
            &completed[0]["reasoning_effort"],
            &completed[0]["response_id"],
            &completed[0]["token_usage"]["total_tokens"]
        ),
        (
            &request["model"],
            &request["reasoning"]["effort"],
            &Value::from("usage"),
            &Value::from(42)
        ),
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reasoning_effort_override_preserves_prefix_and_only_appends_on_change()
-> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));
    let server = responses::start_mock_server().await;
    let mut mocks = Vec::new();
    for id in ["first", "changed", "unchanged", "lowered"] {
        mocks.push(
            responses::mount_sse_once(&server, responses::sse(vec![responses::ev_completed(id)]))
                .await,
        );
    }
    let test = override_builder().build_with_auto_env(&server).await?;
    test.submit_text_turn("first message").await?;
    submit_thread_settings(
        &test.codex,
        ThreadSettingsOverrides {
            effort: Some(Some(ReasoningEffort::High)),
            ..Default::default()
        },
    )
    .await?;
    test.submit_text_turn("second message").await?;
    test.submit_text_turn("third message").await?;
    submit_thread_settings(
        &test.codex,
        ThreadSettingsOverrides {
            effort: Some(Some(ReasoningEffort::Low)),
            ..Default::default()
        },
    )
    .await?;
    test.submit_text_turn("fourth message").await?;

    let requests = mocks
        .iter()
        .map(responses::ResponseMock::single_request)
        .collect::<Vec<_>>();
    let inputs = requests
        .iter()
        .map(|request| {
            responses::strip_response_item_ids_from_json(responses::strip_metadata_from_json(
                Value::Array(request.input()),
            ))
            .as_array()
            .expect("input array")
            .clone()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        requests
            .iter()
            .map(|request| request.body_json()["reasoning"]["effort"].clone())
            .collect::<Vec<_>>(),
        vec![
            Value::from("medium"),
            Value::from("high"),
            Value::from("high"),
            Value::from("low"),
        ],
    );
    let cache_keys = requests
        .iter()
        .map(|request| request.body_json()["prompt_cache_key"].clone())
        .collect::<Vec<_>>();
    assert_eq!(cache_keys, vec![cache_keys[0].clone(); requests.len()]);
    assert_eq!(
        inputs[0][inputs[0].len() - 2..],
        [
            message("user", "first message"),
            effort_update(ReasoningEffort::Medium)
        ],
    );
    let mut expected = inputs[0].clone();
    expected.extend([
        message("user", "second message"),
        effort_update(ReasoningEffort::High),
    ]);
    assert_eq!(inputs[1], expected);
    expected.push(message("user", "third message"));
    assert_eq!(inputs[2], expected);
    expected.extend([
        message("user", "fourth message"),
        effort_update(ReasoningEffort::Low),
    ]);
    assert_eq!(inputs[3], expected);
    Ok(())
}

#[test_case(ReasoningEffort::High, ReasoningEffort::High, ReasoningEffort::Low; "astra manual high is preserved")]
#[test_case(ReasoningEffort::XHigh, ReasoningEffort::XHigh, ReasoningEffort::Low; "astra manual xhigh is preserved")]
#[test_case(ReasoningEffort::Max, ReasoningEffort::Max, ReasoningEffort::Low; "astra manual max is preserved")]
#[test_case(ReasoningEffort::Ultra, ReasoningEffort::Low, ReasoningEffort::Low; "astra ultra respects configured low")]
#[test_case(ReasoningEffort::Ultra, ReasoningEffort::High, ReasoningEffort::High; "astra ultra respects configured high")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reasoning_effort_override_normalizes_astra_before_comparing_updates(
    initial: ReasoningEffort,
    resolved: ReasoningEffort,
    ultra_reasoning_effort: ReasoningEffort,
) -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));
    let server = responses::start_mock_server().await;
    let first = responses::mount_sse_once(
        &server,
        responses::sse(vec![responses::ev_completed("first")]),
    )
    .await;
    let second = responses::mount_sse_once(
        &server,
        responses::sse(vec![responses::ev_completed("second")]),
    )
    .await;
    let test = override_builder()
        .with_model_info_override("gpt-6-astra", |model| model.use_responses_lite = true)
        .with_config(move |config| {
            config.model_reasoning_effort = Some(initial);
            config.ultra_reasoning_effort = ultra_reasoning_effort;
        })
        .build_with_auto_env(&server)
        .await?;
    test.submit_text_turn("initial selection").await?;
    submit_thread_settings(
        &test.codex,
        ThreadSettingsOverrides {
            effort: Some(Some(resolved.clone())),
            ..Default::default()
        },
    )
    .await?;
    test.submit_text_turn("equivalent explicit effort").await?;
    let requests = [first.single_request(), second.single_request()];
    assert_eq!(
        requests.each_ref().map(effort_updates),
        [
            vec![effort_update(resolved.clone())],
            vec![effort_update(resolved.clone())]
        ]
    );
    assert_eq!(
        requests.map(|request| request.body_json()["reasoning"]["effort"].clone()),
        [
            serde_json::to_value(&resolved)?,
            serde_json::to_value(&resolved)?
        ]
    );
    Ok(())
}

#[derive(Clone, Copy)]
enum OverrideUnavailable {
    FeatureDisabled,
    NonOpenAiProvider,
    ResponsesLiteDisabled,
    CustomEffort,
}

#[test_case(OverrideUnavailable::CustomEffort; "custom effort")]
#[test_case(OverrideUnavailable::FeatureDisabled; "feature disabled")]
#[test_case(OverrideUnavailable::NonOpenAiProvider; "unsupported provider")]
#[test_case(OverrideUnavailable::ResponsesLiteDisabled; "responses lite disabled")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reasoning_effort_override_unavailable_uses_request_effort(
    unavailable: OverrideUnavailable,
) -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));
    let server = responses::start_mock_server().await;
    let first = responses::mount_sse_once(
        &server,
        responses::sse(vec![responses::ev_completed("first")]),
    )
    .await;
    let second = responses::mount_sse_once(
        &server,
        responses::sse(vec![responses::ev_completed("second")]),
    )
    .await;
    let custom = ReasoningEffort::Custom("vendor-effort".to_string());
    let initial = if matches!(unavailable, OverrideUnavailable::CustomEffort) {
        custom.clone()
    } else {
        ReasoningEffort::Medium
    };
    let next = if matches!(unavailable, OverrideUnavailable::CustomEffort) {
        custom
    } else {
        ReasoningEffort::High
    };
    let mut builder = match unavailable {
        OverrideUnavailable::CustomEffort => {
            let effort = initial.clone();
            override_builder()
                .with_config(move |config| config.model_reasoning_effort = Some(effort))
        }
        OverrideUnavailable::FeatureDisabled => override_builder().with_config(|config| {
            config
                .features
                .disable(Feature::ReasoningEffortOverride)
                .expect("disable overrides");
        }),
        OverrideUnavailable::NonOpenAiProvider => override_builder().with_config(|config| {
            config.model_provider.name = "unsupported provider".into();
        }),
        OverrideUnavailable::ResponsesLiteDisabled => override_builder()
            .with_model_info_override("gpt-5.4", |model| model.use_responses_lite = false),
    };
    let test = builder.build_with_auto_env(&server).await?;
    test.submit_text_turn("first").await?;
    submit_thread_settings(
        &test.codex,
        ThreadSettingsOverrides {
            effort: Some(Some(next.clone())),
            ..Default::default()
        },
    )
    .await?;
    test.submit_text_turn("second").await?;
    let requests = [first.single_request(), second.single_request()];
    assert_eq!(
        requests.each_ref().map(effort_updates),
        [Vec::<Value>::new(), Vec::new()]
    );
    assert_eq!(
        requests.map(|request| request.body_json()["reasoning"]["effort"].clone()),
        [serde_json::to_value(initial)?, serde_json::to_value(next)?],
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reasoning_effort_override_rollback_and_resume_use_surviving_trusted_history()
-> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));
    let server = responses::start_mock_server().await;
    let mut mocks = Vec::new();
    for id in ["first", "removed", "resumed", "unchanged"] {
        mocks.push(
            responses::mount_sse_once(&server, responses::sse(vec![responses::ev_completed(id)]))
                .await,
        );
    }
    let test = override_builder().build_with_auto_env(&server).await?;
    test.submit_text_turn("keep medium").await?;
    submit_thread_settings(
        &test.codex,
        ThreadSettingsOverrides {
            effort: Some(Some(ReasoningEffort::High)),
            ..Default::default()
        },
    )
    .await?;
    test.submit_text_turn("remove high").await?;
    test.codex
        .submit(Op::ThreadRollback { num_turns: 1 })
        .await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::ThreadRolledBack(_))
    })
    .await;
    let resumed = override_builder()
        .with_config(|config| config.model_reasoning_effort = Some(ReasoningEffort::High))
        .restart(&server, &test)
        .await?;
    resumed
        .submit_text_turn("restore high after rollback")
        .await?;
    resumed.submit_text_turn("keep high").await?;
    let medium = effort_update(ReasoningEffort::Medium);
    let high = effort_update(ReasoningEffort::High);
    assert_eq!(
        mocks
            .iter()
            .map(|mock| effort_updates(&mock.single_request()))
            .collect::<Vec<_>>(),
        vec![
            vec![medium.clone()],
            vec![medium.clone(), high.clone()],
            vec![medium.clone(), high.clone()],
            vec![medium, high],
        ]
    );
    Ok(())
}
