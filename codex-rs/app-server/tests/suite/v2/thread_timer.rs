use std::time::Duration;

use anyhow::Result;
use app_test_support::MockResponsesConfig;
use app_test_support::TestAppServer;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadTimerClearParams;
use codex_app_server_protocol::ThreadTimerClearResponse;
use codex_app_server_protocol::ThreadTimerListParams;
use codex_app_server_protocol::ThreadTimerListResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::UserInput;
use core_test_support::responses;
use pretty_assertions::assert_eq;
use tempfile::TempDir;
use tokio::time::timeout;

const READ_TIMEOUT: Duration = Duration::from_secs(/*secs*/ 10);

#[tokio::test]
async fn timer_list_and_clear_use_public_thread_rpc() -> Result<()> {
    let server = responses::start_mock_server().await;
    responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse(vec![
                responses::ev_response_created("timer-set-response"),
                responses::ev_function_call_with_namespace(
                    "timer-set-call",
                    "timer",
                    "set",
                    r#"{"delay_ms":60000,"reminder":"check the oven"}"#,
                ),
                responses::ev_completed("timer-set-response"),
            ]),
            responses::sse(vec![
                responses::ev_response_created("timer-final-response"),
                responses::ev_assistant_message("timer-confirmation", "Timer scheduled"),
                responses::ev_completed("timer-final-response"),
            ]),
        ],
    )
    .await;

    let codex_home = TempDir::new()?;
    MockResponsesConfig::new(&server.uri()).write(codex_home.path())?;
    let mut app = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .build_initialized()
        .await?;
    let thread = app
        .start_thread(ThreadStartParams {
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?
        .thread;

    let _: TurnStartResponse = app
        .request(|request_id| ClientRequest::TurnStart {
            request_id,
            params: TurnStartParams {
                thread_id: thread.id.clone(),
                client_user_message_id: None,
                input: vec![UserInput::Text {
                    text: "Remind me later".to_string(),
                    text_elements: Vec::new(),
                }],
                ..Default::default()
            },
        })
        .await?;
    timeout(
        READ_TIMEOUT,
        app.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let listed: ThreadTimerListResponse = app
        .request(|request_id| ClientRequest::ThreadTimerList {
            request_id,
            params: ThreadTimerListParams {
                thread_id: thread.id.clone(),
                cursor: None,
                limit: None,
            },
        })
        .await?;
    assert_eq!(listed.data.len(), 1);
    assert_eq!(listed.data[0].reminder, "check the oven");
    assert_eq!(listed.data[0].interval_ms, None);
    assert_eq!(listed.next_cursor, None);

    let cleared: ThreadTimerClearResponse = app
        .request(|request_id| ClientRequest::ThreadTimerClear {
            request_id,
            params: ThreadTimerClearParams {
                thread_id: thread.id.clone(),
            },
        })
        .await?;
    assert_eq!(cleared.cleared, 1);

    let listed_after_clear: ThreadTimerListResponse = app
        .request(|request_id| ClientRequest::ThreadTimerList {
            request_id,
            params: ThreadTimerListParams {
                thread_id: thread.id,
                cursor: None,
                limit: None,
            },
        })
        .await?;
    assert_eq!(listed_after_clear.data, Vec::new());
    assert_eq!(listed_after_clear.next_cursor, None);

    Ok(())
}
