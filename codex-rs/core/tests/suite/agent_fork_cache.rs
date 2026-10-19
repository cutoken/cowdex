use anyhow::Result;
use codex_features::Feature;
use codex_protocol::protocol::ThreadHistoryMode;
use core_test_support::responses;
use core_test_support::test_codex::test_codex;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use std::time::Duration;

const CHILD_TASK: &str = "inspect the inherited cached tool record";

fn is_child_request(request: &wiremock::Request) -> bool {
    serde_json::from_slice::<Value>(&request.body).is_ok_and(|body| is_child_body(&body))
}

fn is_child_body(body: &Value) -> bool {
    body["input"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["type"] == "agent_message" && item.to_string().contains(CHILD_TASK))
    })
}

#[test_case::test_case(ThreadHistoryMode::Legacy; "legacy")]
#[test_case::test_case(ThreadHistoryMode::Paginated; "paginated")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_agent_fork_preserves_request_prefix_and_code_mode_records(
    history_mode: ThreadHistoryMode,
) -> Result<()> {
    let server = responses::start_mock_server().await;
    responses::mount_sse_once(
        &server,
        responses::sse(vec![
            responses::ev_response_created("seed-tool"),
            responses::ev_custom_tool_call("seed-tool-call", "exec", "text('cached-tool-record')"),
            responses::ev_completed("seed-tool"),
        ]),
    )
    .await;
    responses::mount_sse_once(
        &server,
        responses::sse(vec![
            responses::ev_response_created("seed-answer"),
            responses::ev_assistant_message("seed-commentary", "historical parent commentary"),
            responses::ev_completed("seed-answer"),
        ]),
    )
    .await;
    let spawn_args = json!({
        "task_name": "cached_worker", "message": CHILD_TASK, "fork_turns": "all"
    });
    let spawn_mock = responses::mount_sse_once(
        &server,
        responses::sse(vec![
            responses::ev_response_created("spawn"),
            responses::ev_function_call_with_namespace(
                "spawn-call",
                "collaboration",
                "spawn_agent",
                &spawn_args.to_string(),
            ),
            responses::ev_completed("spawn"),
        ]),
    )
    .await;
    let child_mock = responses::mount_sse_once_match(
        &server,
        is_child_request,
        responses::sse(vec![
            responses::ev_response_created("child"),
            responses::ev_assistant_message("child-answer", "child finished"),
            responses::ev_completed("child"),
        ]),
    )
    .await;
    responses::mount_sse_once_match(
        &server,
        |request: &wiremock::Request| !is_child_request(request),
        responses::sse(vec![
            responses::ev_response_created("parent-followup"),
            responses::ev_assistant_message("parent-answer", "parent finished"),
            responses::ev_completed("parent-followup"),
        ]),
    )
    .await;
    let mut builder = test_codex()
        .with_history_mode(history_mode)
        .with_config(|config| {
            let _ = config.features.enable(Feature::Collab);
            let _ = config.features.enable(Feature::MultiAgentV2);
            let _ = config.features.enable(Feature::CodeMode);
            let _ = config.features.disable(Feature::EnableRequestCompression);
            config.multi_agent_v2.preserve_fork_prefix = true;
            config.multi_agent_v2.subagent_developer_instructions =
                Some("Distinct child instructions.".to_string());
        });
    let test = builder.build_with_auto_env(&server).await?;
    test.submit_turn("seed the tool history").await?;
    test.submit_turn("spawn a cached worker").await?;
    tokio::time::timeout(Duration::from_secs(10), async {
        while !child_mock
            .requests()
            .iter()
            .any(|request| is_child_body(&request.body_json()))
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    let parent_request = spawn_mock
        .requests()
        .into_iter()
        .find(|request| {
            request
                .message_input_texts("user")
                .iter()
                .any(|text| text == "spawn a cached worker")
                && !request
                    .input()
                    .iter()
                    .any(|item| item["call_id"] == "spawn-call")
        })
        .expect("parent spawn request");
    let parent_input = parent_request.input();
    let child_request = child_mock
        .requests()
        .into_iter()
        .find(|request| is_child_body(&request.body_json()))
        .expect("child request");
    let child_input = child_request.input();
    assert_eq!(
        child_request.body_json()["prompt_cache_key"],
        parent_request.body_json()["prompt_cache_key"]
    );
    assert_eq!(
        child_request.header("session-id"),
        parent_request.header("session-id")
    );
    assert_eq!(
        child_input.get(..parent_input.len()),
        Some(parent_input.as_slice())
    );
    assert!(
        child_input
            .iter()
            .any(|item| item["type"] == "custom_tool_call")
    );
    assert!(
        child_input
            .iter()
            .any(|item| item["type"] == "custom_tool_call_output")
    );
    assert!(
        child_input
            .iter()
            .any(|item| item["type"] == "custom_tool_call_output"
                && item["call_id"] == "seed-tool-call"
                && item.to_string().contains("cached-tool-record"))
    );
    assert_eq!(
        child_request
            .message_input_texts("developer")
            .iter()
            .filter(|text| text.contains("Distinct child instructions."))
            .count(),
        1,
    );
    assert!(
        child_input
            .iter()
            .any(|item| item["call_id"] == "spawn-call" && item["type"] == "function_call_output")
    );
    Ok(())
}
