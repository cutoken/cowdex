use anyhow::Result;
use codex_features::Feature;
use codex_login::CodexAuth;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use core_test_support::responses;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::json;
use test_case::test_case;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Legacy,
    V2,
    Local,
}

#[derive(Clone, Copy, PartialEq)]
enum Trigger {
    Manual,
    Auto,
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    Astra,
    SolHigh,
    SolXHigh,
    SolMax,
    SolUltraHigh,
    SolUltraLow,
    SolMedium,
    SolLow,
    DefaultOff,
}

#[test_case(Mode::V2, Selection::Astra, Trigger::Manual; "astra v2")]
#[test_case(Mode::Legacy, Selection::Astra, Trigger::Manual; "astra legacy")]
#[test_case(Mode::Local, Selection::Astra, Trigger::Manual; "astra local")]
#[test_case(Mode::V2, Selection::Astra, Trigger::Auto; "astra automatic")]
#[test_case(Mode::V2, Selection::SolHigh, Trigger::Manual; "sol high")]
#[test_case(Mode::V2, Selection::SolXHigh, Trigger::Manual; "sol xhigh")]
#[test_case(Mode::V2, Selection::SolMax, Trigger::Manual; "sol max")]
#[test_case(Mode::V2, Selection::SolUltraHigh, Trigger::Manual; "sol ultra high")]
#[test_case(Mode::V2, Selection::SolUltraLow, Trigger::Manual; "sol ultra low unchanged")]
#[test_case(Mode::V2, Selection::SolMedium, Trigger::Manual; "sol medium unchanged")]
#[test_case(Mode::V2, Selection::SolLow, Trigger::Manual; "sol low unchanged")]
#[test_case(Mode::V2, Selection::DefaultOff, Trigger::Manual; "disabled by default")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compact_with_lower_preserves_working_selection(
    mode: Mode,
    selection: Selection,
    trigger: Trigger,
) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let server = responses::start_mock_server().await;
    let (model, effort, ultra_effort, normal_effort, compact_model, compact_effort) =
        match selection {
            Selection::Astra => (
                "gpt-6-astra",
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                "low",
                "gpt-5.6-sol",
                "medium",
            ),
            Selection::SolHigh => (
                "gpt-5.6-sol",
                ReasoningEffort::High,
                ReasoningEffort::Medium,
                "high",
                "gpt-5.6-sol",
                "medium",
            ),
            Selection::SolXHigh => (
                "gpt-5.6-sol",
                ReasoningEffort::XHigh,
                ReasoningEffort::Medium,
                "xhigh",
                "gpt-5.6-sol",
                "medium",
            ),
            Selection::SolMax => (
                "gpt-5.6-sol",
                ReasoningEffort::Max,
                ReasoningEffort::Medium,
                "max",
                "gpt-5.6-sol",
                "medium",
            ),
            Selection::SolUltraHigh => (
                "gpt-5.6-sol",
                ReasoningEffort::Ultra,
                ReasoningEffort::High,
                "high",
                "gpt-5.6-sol",
                "medium",
            ),
            Selection::SolUltraLow => (
                "gpt-5.6-sol",
                ReasoningEffort::Ultra,
                ReasoningEffort::Low,
                "low",
                "gpt-5.6-sol",
                "low",
            ),
            Selection::SolMedium => (
                "gpt-5.6-sol",
                ReasoningEffort::Medium,
                ReasoningEffort::Medium,
                "medium",
                "gpt-5.6-sol",
                "medium",
            ),
            Selection::SolLow => (
                "gpt-5.6-sol",
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                "low",
                "gpt-5.6-sol",
                "low",
            ),
            Selection::DefaultOff => (
                "gpt-6-astra",
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                "low",
                "gpt-6-astra",
                "low",
            ),
        };
    let test = test_codex()
        .with_auth(CodexAuth::create_dummy_chatgpt_auth_for_testing())
        .with_model_info_override(model, |info| {
            info.supports_reasoning_summary_parameter = true;
            info.use_responses_lite = true;
        })
        .with_config(move |config| {
            if selection != Selection::DefaultOff {
                config.compact_with_lower = true;
            }
            config.model_reasoning_effort = Some(effort);
            config.ultra_reasoning_effort = ultra_effort;
            if selection == Selection::Astra {
                config.model_context_window = Some(400_000);
            }
            if selection == Selection::SolHigh {
                config
                    .features
                    .enable(Feature::ReasoningEffortOverride)
                    .expect("enable effort history updates");
            }
            config.model_auto_compact_token_limit = match trigger {
                Trigger::Manual => None,
                Trigger::Auto => Some(200),
            };
            match mode {
                Mode::V2 => {
                    config
                        .features
                        .enable(Feature::RemoteCompactionV2)
                        .expect("enable remote compaction v2");
                }
                Mode::Legacy => {
                    config
                        .features
                        .disable(Feature::RemoteCompactionV2)
                        .expect("disable remote compaction v2");
                }
                Mode::Local => config.model_provider.name = "OpenAI (test)".to_string(),
            }
        })
        .build_with_auto_env(&server)
        .await?;
    let before = responses::mount_sse_once(
        &server,
        responses::sse(vec![
            responses::ev_assistant_message("before-message", "Remember the project decision."),
            responses::ev_completed_with_tokens("before", /*total_tokens*/ 500),
        ]),
    )
    .await;
    test.submit_text_turn("Use blue for the project.").await?;
    let compact = match mode {
        Mode::Legacy => responses::mount_compact_user_history_with_summary_once(&server, "Project uses blue.").await,
        Mode::V2 => responses::mount_sse_once(&server, responses::sse(vec![
            json!({"type": "response.output_item.done", "item": {"type": "compaction", "encrypted_content": "project-blue-summary"}}),
            responses::ev_completed("compact"),
        ])).await,
        Mode::Local => responses::mount_sse_once(&server, responses::sse(vec![
            responses::ev_assistant_message("summary", "Project uses blue."),
            responses::ev_completed("compact"),
        ])).await,
    };
    if trigger == Trigger::Manual {
        test.codex.submit(Op::Compact).await?;
        wait_for_event(&test.codex, |event| {
            matches!(event, EventMsg::TurnComplete(_))
        })
        .await;
    }
    let after = responses::mount_sse_once(
        &server,
        responses::sse(vec![
            responses::ev_assistant_message("after-message", "Blue."),
            responses::ev_completed("after"),
        ]),
    )
    .await;
    test.submit_text_turn("What color did we choose?").await?;
    let requests = [
        before.single_request(),
        compact.single_request(),
        after.single_request(),
    ];
    if selection == Selection::SolHigh {
        let updates = requests
            .iter()
            .map(|request| {
                request
                    .input()
                    .into_iter()
                    .filter(|item| item["type"] == "configuration_update")
                    .map(|item| item["reasoning"]["effort"].clone())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            updates,
            vec![
                vec![json!("high")],
                vec![json!("high"), json!("medium")],
                vec![json!("high")]
            ]
        );
    }
    let actual = requests
        .iter()
        .map(|request| {
            let body = request.body_json();
            json!({"model": body["model"], "effort": body["reasoning"]["effort"]})
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            json!({"model": model, "effort": normal_effort}),
            json!({"model": compact_model, "effort": compact_effort}),
            json!({"model": model, "effort": normal_effort}),
        ]
    );
    Ok(())
}
