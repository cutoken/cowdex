use anyhow::Result;
use codex_core::StartThreadOptions;
use codex_core::TurnInputRequest;
use codex_features::Feature;
use codex_history::InitialHistory;
use codex_history::RolloutItem;
use codex_login::CodexAuth;
use codex_model_provider_info::built_in_model_providers;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::user_input::UserInput;
use core_test_support::responses;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::json;
use test_case::test_case;

#[derive(Clone, Copy)]
enum CompactionMode {
    Local,
    RemoteV1,
    RemoteV2,
}

#[test_case(CompactionMode::Local; "local")]
#[test_case(CompactionMode::RemoteV1; "remote_v1")]
#[test_case(CompactionMode::RemoteV2; "remote_v2")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compaction_chronology_preserves_user_age_and_completed_work(
    mode: CompactionMode,
) -> Result<()> {
    let server = responses::start_mock_server().await;
    let sent_at = chrono::Utc::now().timestamp() - 7200;
    let old_user = json!({
        "type": "message", "role": "user",
        "content": [{"type": "input_text", "text": "Implement the requested change"}],
        "internal_chat_message_metadata_passthrough": {"create_time": sent_at}
    });
    let completed_work = json!({
        "type": "message", "role": "assistant",
        "content": [{"type": "output_text", "text": "The requested change is implemented and verified."}]
    });
    let summary = responses::sse(vec![
        responses::ev_assistant_message(
            "summary",
            "The requested change is already implemented and verified. Nothing remains for that request.",
        ),
        responses::ev_completed("compact"),
    ]);
    let compact_response = match mode {
        CompactionMode::Local => Some(summary),
        CompactionMode::RemoteV1 => None,
        CompactionMode::RemoteV2 => Some(responses::sse(vec![
            json!({"type": "response.output_item.done", "item": {
                "type": "compaction", "encrypted_content": "COMPLETED_WORK_SUMMARY"
            }}),
            responses::ev_completed("compact"),
        ])),
    };
    let _remote_compact = match mode {
        CompactionMode::RemoteV1 => Some(responses::mount_compact_json_once(&server, json!({"output": [
            old_user.clone(), {"type": "compaction", "encrypted_content": "COMPLETED_WORK_SUMMARY"}
        ]})).await),
        CompactionMode::Local | CompactionMode::RemoteV2 => None,
    };
    let response_mock = responses::mount_sse_sequence(
        &server,
        compact_response
            .into_iter()
            .chain([responses::sse_completed("follow-up")])
            .collect(),
    )
    .await;
    let mut provider = built_in_model_providers(/*openai_base_url*/ None)["openai"].clone();
    provider.base_url = Some(format!("{}/v1", server.uri()));
    provider.supports_websockets = false;
    if matches!(mode, CompactionMode::Local) {
        provider.name = "Local compaction test provider".to_string();
    }
    let mut builder = test_codex()
        .with_auth(CodexAuth::create_dummy_chatgpt_auth_for_testing())
        .with_config(move |config| {
            config.model_provider = provider;
            if matches!(mode, CompactionMode::RemoteV1) {
                config
                    .features
                    .disable(Feature::RemoteCompactionV2)
                    .unwrap();
            }
        });
    let test = builder.build_with_auto_env(&server).await?;
    let thread = test
        .thread_manager
        .start_thread(StartThreadOptions {
            initial_history: InitialHistory::Forked(
                vec![old_user, completed_work]
                    .into_iter()
                    .map(|item| {
                        serde_json::from_value::<ResponseItem>(item)
                            .map(|item| RolloutItem::ResponseItem(item.into()))
                    })
                    .collect::<serde_json::Result<Vec<_>>>()?,
            ),
            ..StartThreadOptions::new(test.config.clone())
        })
        .await?
        .thread;
    thread.submit(Op::Compact).await?;
    wait_for_event(&thread, |event| matches!(event, EventMsg::TurnComplete(_))).await;
    thread
        .start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
            text: "Now handle the next task".to_string(),
            text_elements: Vec::new(),
        }]))
        .await?;
    wait_for_event(&thread, |event| matches!(event, EventMsg::TurnComplete(_))).await;
    let requests = response_mock.requests();
    let follow_up = requests.last().unwrap();
    let developer_messages = follow_up.message_input_texts("developer");
    let notes: Vec<_> = developer_messages
        .iter()
        .filter(|text| text.starts_with("<compaction_chronology>"))
        .collect();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].contains("2h 0m"));
    assert!(notes[0].contains("rather than repeating completed actions"));
    assert_eq!(
        follow_up
            .message_input_texts("user")
            .iter()
            .filter(|text| text.as_str() == "Implement the requested change")
            .count(),
        1
    );
    assert!(
        follow_up
            .body_json()
            .to_string()
            .contains("COMPLETED_WORK_SUMMARY")
            || follow_up
                .body_json()
                .to_string()
                .contains("already implemented and verified")
    );
    let compact_request = match mode {
        CompactionMode::Local | CompactionMode::RemoteV2 => requests[0].body_json(),
        CompactionMode::RemoteV1 => _remote_compact.as_ref().unwrap().requests()[0].body_json(),
    };
    assert!(
        compact_request
            .to_string()
            .contains("The requested change is implemented and verified.")
    );
    Ok(())
}
