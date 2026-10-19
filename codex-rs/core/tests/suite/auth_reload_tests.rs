use anyhow::Result;
use base64::Engine;
use codex_core::TurnInputRequest;
use codex_login::CodexAuth;
use codex_protocol::protocol::EventMsg;
use codex_protocol::user_input::UserInput;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::responses::start_websocket_server;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::path::Path;
use test_case::test_case;
use wiremock::Mock;
use wiremock::ResponseTemplate;
use wiremock::matchers::header;
use wiremock::matchers::method;
use wiremock::matchers::path;

fn save_new_login(home: &Path, account_id: &str) -> Result<()> {
    let claims =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": account_id,
                "chatgpt_plan_type": "pro"
            }
        }))?);
    std::fs::write(
        home.join("auth.json"),
        serde_json::to_vec(&json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": format!("e30.{claims}.test"),
                "access_token": "new-test-token",
                "refresh_token": "new-test-refresh-token",
                "account_id": account_id
            },
            "last_refresh": chrono::Utc::now()
        }))?,
    )?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn user_turn_reloads_saved_login_after_usage_limit() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let server = start_mock_server().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer Access Token"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "error": {
                "type": "usage_limit_reached",
                "message": "limit reached",
                "plan_type": "pro",
                "resets_at": 2000000000
            }
        })))
        .expect(1)
        .mount(&server)
        .await;
    let mut builder = test_codex().with_auth(CodexAuth::create_dummy_chatgpt_auth_for_testing());
    let test = builder.build_with_auto_env(&server).await?;
    test.codex
        .start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
            text: "hello".into(),
            text_elements: Vec::new(),
        }]))
        .await?;
    let event = wait_for_event(&test.codex, |event| matches!(event, EventMsg::Error(_))).await;
    let EventMsg::Error(error) = event else {
        unreachable!();
    };
    assert!(error.message.contains("usage limit"), "{error:?}");
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;

    save_new_login(&test.config.codex_home, "new-account")?;
    let response = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-2"), ev_completed("resp-2")]),
    )
    .await;
    test.submit_turn("continue").await?;
    let request = response.single_request();
    assert_eq!(
        (
            request.header("authorization"),
            request.header("chatgpt-account-id")
        ),
        (
            Some("Bearer new-test-token".into()),
            Some("new-account".into())
        )
    );
    server.verify().await;
    Ok(())
}

#[test_case("new-account"; "different_account")]
#[test_case("account_id"; "same_account_new_token")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn user_turn_reconnects_websocket_only_when_saved_login_changes(
    account_id: &str,
) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let server = start_websocket_server(vec![
        vec![
            vec![ev_response_created("warm-1"), ev_completed("warm-1")],
            vec![ev_response_created("resp-1"), ev_completed("resp-1")],
            vec![ev_response_created("resp-2"), ev_completed("resp-2")],
        ],
        vec![vec![ev_response_created("resp-3"), ev_completed("resp-3")]],
    ])
    .await;
    let mut builder = test_codex().with_auth(CodexAuth::create_dummy_chatgpt_auth_for_testing());
    let test = builder.build_with_websocket_server(&server).await?;
    test.submit_turn("first turn").await?;
    test.submit_turn("unchanged login").await?;
    assert_eq!(server.handshakes().len(), 1);

    save_new_login(&test.config.codex_home, account_id)?;
    // A sibling session may notice the new login before this session's next turn.
    test.thread_manager.auth_manager().reload().await;
    test.submit_turn("use new login").await?;
    let headers = server
        .handshakes()
        .iter()
        .map(|handshake| {
            (
                handshake.header("authorization"),
                handshake.header("chatgpt-account-id"),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        headers,
        vec![
            (
                Some("Bearer Access Token".into()),
                Some("account_id".into())
            ),
            (
                Some("Bearer new-test-token".into()),
                Some(account_id.into())
            )
        ]
    );
    let connections = server.connections();
    assert_eq!(
        connections.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![3, 1]
    );
    assert_eq!(
        connections[1][0].body_json().get("previous_response_id"),
        None
    );
    server.shutdown().await;
    Ok(())
}
