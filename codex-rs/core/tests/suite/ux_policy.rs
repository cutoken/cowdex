use anyhow::Result;
use codex_features::Feature;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use core_test_support::responses;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::json;

fn policy(request: &responses::ResponsesRequest) -> String {
    let messages = request.message_input_texts("developer");
    let policies: Vec<_> = messages
        .iter()
        .filter_map(|text| text.split_once("<ux_policy>"))
        .filter_map(|(_, text)| text.split_once("</ux_policy>"))
        .map(|(policy, _)| policy.to_string())
        .collect();
    assert_eq!(policies.len(), 1);
    policies[0].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ux_policy_is_stable_and_survives_compaction_and_resume() -> Result<()> {
    let server = responses::start_mock_server().await;
    let requests = responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse_completed("first"),
            responses::sse_completed("second"),
            responses::sse(vec![
                responses::ev_assistant_message("summary", "Continue the UI work."),
                responses::ev_completed("compact"),
            ]),
            responses::sse_completed("after-compact"),
            responses::sse_completed("after-resume"),
        ],
    )
    .await;
    let builder = || {
        test_codex().with_config(|config| {
            config.model_provider.name = "Local summary test provider".into();
        })
    };
    let test = builder().build_with_auto_env(&server).await?;
    test.submit_text_turn("Design the interface").await?;
    test.submit_text_turn("Continue implementing the interface")
        .await?;
    test.codex.submit(Op::Compact).await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    test.submit_text_turn("Continue after compaction").await?;
    let resumed = builder().restart(&server, &test).await?;
    resumed.submit_text_turn("Continue after resume").await?;
    let requests = requests.requests();
    let original = policy(&requests[0]);
    let installed = test.config.codex_home.join("policies/cowdex/ux_policy.md");
    let contents = tokio::fs::read_to_string(&installed).await?;
    assert!(contents.contains("Bug fixes are exempt"));
    assert!(contents.contains("Do not be wordy"));
    assert!(contents.contains("Do not introduce Material Design or flat themes"));
    assert!(original.contains(installed.to_string_lossy().as_ref()));
    assert!(original.len() < 1000);
    for request in &requests[1..] {
        assert_eq!(policy(request), original);
        assert!(
            !request
                .body_json()
                .to_string()
                .contains("Recognize → choose → see the result")
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ux_agent_receives_same_bundled_policy_as_main_agent() -> Result<()> {
    let server = responses::start_mock_server().await;
    let root = responses::mount_sse_once_match(
        &server,
        |request: &wiremock::Request| request.body.windows(b"Create the product screen".len()).any(|text| text == b"Create the product screen"),
        responses::sse(vec![
            responses::ev_function_call_with_namespace("spawn-ux", "collaboration", "spawn_agent", &json!({
                "task_name": "design", "message": "Review the assigned interface", "agent_type": "ui_ux", "fork_turns": "none"
            }).to_string()),
            responses::ev_completed("spawn"),
        ]),
    ).await;
    let worker = responses::mount_sse_once_match(
        &server,
        |request: &wiremock::Request| {
            request
                .body
                .windows(b"Review the assigned interface".len())
                .any(|text| text == b"Review the assigned interface")
                && !request
                    .body
                    .windows(b"spawn-ux".len())
                    .any(|text| text == b"spawn-ux")
        },
        responses::sse_completed("ux-complete"),
    )
    .await;
    responses::mount_sse_once_match(
        &server,
        |request: &wiremock::Request| {
            request
                .body
                .windows(b"function_call_output".len())
                .any(|text| text == b"function_call_output")
        },
        responses::sse_completed("root-complete"),
    )
    .await;
    let mut builder = test_codex().with_config(|config| {
        config.features.enable(Feature::Collab).unwrap();
        config.features.enable(Feature::MultiAgentV2).unwrap();
    });
    let test = builder.build_with_auto_env(&server).await?;
    test.submit_text_turn("Create the product screen").await?;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while worker.requests().is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await?;
    let worker_request = worker.single_request();
    assert_eq!(policy(&worker_request), policy(&root.single_request()));
    assert!(policy(&worker_request).contains("the UX agent must enforce it"));
    Ok(())
}
