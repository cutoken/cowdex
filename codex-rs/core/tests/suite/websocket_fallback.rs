use anyhow::Result;
use codex_core::TurnInputRequest;
use codex_model_provider_info::WireApi;
use codex_protocol::config_types::CollaborationMode;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::Settings;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ThreadSettingsOverrides;
use codex_protocol::user_input::UserInput;
use core_test_support::TempDirExt;
use core_test_support::responses;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::TestCodex;
use core_test_support::test_codex::local_selections;
use core_test_support::test_codex::test_codex;
use core_test_support::test_codex::turn_permission_fields;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use test_case::test_case;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::time::Duration;
use tokio::time::timeout;
use wiremock::Mock;
use wiremock::ResponseTemplate;
use wiremock::http::Method;
use wiremock::matchers::method;
use wiremock::matchers::path_regex;

#[test_case(None; "server_closed")]
#[test_case(Some(503); "server_error")]
#[tokio::test(flavor = "current_thread")]
async fn websocket_failures_fall_back_to_http_and_stay_there(
    server_status: Option<u16>,
) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let websocket_server = responses::start_websocket_server(vec![
        vec![
            vec![ev_response_created("prewarm"), ev_completed("prewarm")],
            vec![server_status.map_or_else(
                || ev_response_created("closed-first"),
                |status| serde_json::json!({
                    "type": "error", "status": status,
                    "error": {"code": "server_is_overloaded", "message": "Temporary server failure"}
                }),
            )],
        ],
        vec![vec![ev_response_created("closed-second")]],
    ])
    .await;
    let http_server = responses::start_mock_server().await;
    let http_responses = mount_sse_sequence(
        &http_server,
        vec![
            responses::sse_completed("recovered"),
            responses::sse_completed("next-turn"),
        ],
    )
    .await;
    // Route both transports through the same endpoint, using the existing scripted servers.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let ws_address = websocket_server
        .uri()
        .trim_start_matches("http://")
        .trim_start_matches("ws://")
        .to_string();
    let http_address = http_server.uri().trim_start_matches("http://").to_string();
    let proxy = tokio::spawn(async move {
        while let Ok((mut incoming, _)) = listener.accept().await {
            let ws_address = ws_address.clone();
            let http_address = http_address.clone();
            tokio::spawn(async move {
                let mut prefix = [0; 3];
                if incoming.peek(&mut prefix).await.is_err() {
                    return;
                }
                let target = if &prefix == b"GET" {
                    ws_address
                } else {
                    http_address
                };
                if let Ok(mut outgoing) = TcpStream::connect(target).await {
                    let _ = tokio::io::copy_bidirectional(&mut incoming, &mut outgoing).await;
                }
            });
        }
    });
    let mut builder = test_codex().with_config(move |config| {
        config.model_provider.base_url = Some(format!("http://{address}/v1"));
        config.model_provider.supports_websockets = true;
        config.model_provider.stream_max_retries = Some(5);
        config.model_provider.request_max_retries = Some(0);
    });
    let test = builder.build_with_auto_env(&http_server).await?;
    test.codex
        .start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
            text: "recover from server closing the websocket".into(),
            text_elements: Vec::new(),
        }]))
        .await?;
    let mut retries = 0;
    let mut fallbacks = 0;
    loop {
        match wait_for_event(&test.codex, |_| true).await {
            EventMsg::StreamError(_) => {
                retries += 1;
                if server_status.is_some() {
                    tokio::time::pause();
                    tokio::time::advance(Duration::from_secs(60)).await;
                    tokio::time::resume();
                }
            }
            EventMsg::Warning(warning)
                if warning.message.contains("Falling back from WebSockets") =>
            {
                fallbacks += 1
            }
            EventMsg::TurnComplete(completed) => {
                assert_eq!(completed.error, None);
                break;
            }
            _ => {}
        }
    }
    test.submit_turn("keep using HTTP").await?;
    let expected_connections = if server_status.is_some() { 1 } else { 2 };
    assert_eq!(websocket_server.handshakes().len(), expected_connections);
    assert_eq!(
        websocket_server
            .connections()
            .iter()
            .map(Vec::len)
            .sum::<usize>(),
        expected_connections + 1
    );
    assert_eq!(http_responses.requests().len(), 2);
    assert_eq!(
        retries,
        usize::from(server_status.is_some() || cfg!(debug_assertions))
    );
    assert_eq!(fallbacks, 1);
    proxy.abort();
    websocket_server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_fallback_switches_to_http_on_upgrade_required_connect() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    Mock::given(method("GET"))
        .and(path_regex(".*/responses$"))
        .respond_with(ResponseTemplate::new(426))
        .mount(&server)
        .await;

    let response_mock = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;

    let mut builder = test_codex().with_config({
        let base_url = format!("{}/v1", server.uri());
        move |config| {
            config.model_provider.base_url = Some(base_url);
            config.model_provider.wire_api = WireApi::Responses;
            config.model_provider.supports_websockets = true;
            // If we don't treat 426 specially, the sampling loop would retry the WebSocket
            // handshake before switching to the HTTP transport.
            config.model_provider.stream_max_retries = Some(2);
            config.model_provider.request_max_retries = Some(0);
        }
    });
    let test = builder.build(&server).await?;

    test.submit_turn("hello").await?;

    let requests = server.received_requests().await.unwrap_or_default();
    let websocket_attempts = requests
        .iter()
        .filter(|req| req.method == Method::GET && req.url.path().ends_with("/responses"))
        .count();
    let http_attempts = requests
        .iter()
        .filter(|req| req.method == Method::POST && req.url.path().ends_with("/responses"))
        .count();

    // The startup prewarm request sees 426 and immediately switches the session to HTTP fallback,
    // so the first turn goes straight to HTTP with no additional websocket connect attempt.
    assert_eq!(websocket_attempts, 1);
    assert_eq!(http_attempts, 1);
    assert_eq!(response_mock.requests().len(), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_fallback_switches_to_http_after_retries_exhausted() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    let response_mock = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;

    let mut builder = test_codex().with_config({
        let base_url = format!("{}/v1", server.uri());
        move |config| {
            config.model_provider.base_url = Some(base_url);
            config.model_provider.wire_api = WireApi::Responses;
            config.model_provider.supports_websockets = true;
            config.model_provider.stream_max_retries = Some(2);
            config.model_provider.request_max_retries = Some(0);
        }
    });
    let test = builder.build(&server).await?;

    test.submit_turn("hello").await?;

    let requests = server.received_requests().await.unwrap_or_default();
    let websocket_attempts = requests
        .iter()
        .filter(|req| req.method == Method::GET && req.url.path().ends_with("/responses"))
        .count();
    let http_attempts = requests
        .iter()
        .filter(|req| req.method == Method::POST && req.url.path().ends_with("/responses"))
        .count();

    // Deferred request prewarm is attempted at startup.
    // The first turn then makes 3 websocket stream attempts (initial try + 2 retries),
    // after which fallback activates and the request is replayed over HTTP.
    assert_eq!(websocket_attempts, 4);
    assert_eq!(http_attempts, 1);
    assert_eq!(response_mock.requests().len(), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_fallback_hides_first_websocket_retry_stream_error() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    let response_mock = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;

    let mut builder = test_codex().with_config({
        let base_url = format!("{}/v1", server.uri());
        move |config| {
            config.model_provider.base_url = Some(base_url);
            config.model_provider.wire_api = WireApi::Responses;
            config.model_provider.supports_websockets = true;
            config.model_provider.stream_max_retries = Some(2);
            config.model_provider.request_max_retries = Some(0);
        }
    });
    let TestCodex {
        codex,
        session_configured,
        cwd,
        ..
    } = builder.build(&server).await?;
    let (sandbox_policy, permission_profile) =
        turn_permission_fields(PermissionProfile::Disabled, cwd.path());

    codex
        .start_or_steer_turn(
            TurnInputRequest::user_input(vec![UserInput::Text {
                text: "hello".into(),
                text_elements: Vec::new(),
            }])
            .with_thread_settings(ThreadSettingsOverrides {
                environments: Some(local_selections(cwd.abs())),
                approval_policy: Some(AskForApproval::Never),
                sandbox_policy: Some(sandbox_policy),
                permission_profile,
                collaboration_mode: Some(CollaborationMode {
                    mode: ModeKind::Default,
                    settings: Settings {
                        model: session_configured.model.clone(),
                        reasoning_effort: None,
                        developer_instructions: None,
                    },
                }),
                ..Default::default()
            }),
        )
        .await?;

    let mut stream_error_messages = Vec::new();
    loop {
        let event = timeout(Duration::from_secs(10), codex.next_event())
            .await
            .expect("timeout waiting for event")
            .expect("event stream ended unexpectedly")
            .msg;
        match event {
            EventMsg::StreamError(e) => stream_error_messages.push(e.message),
            EventMsg::TurnComplete(_) => break,
            _ => {}
        }
    }

    let expected_stream_errors = if cfg!(debug_assertions) {
        vec!["Reconnecting... 1/2", "Reconnecting... 2/2"]
    } else {
        vec!["Reconnecting... 2/2"]
    };
    assert_eq!(stream_error_messages, expected_stream_errors);
    assert_eq!(response_mock.requests().len(), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_fallback_is_sticky_across_turns() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    let response_mock = mount_sse_sequence(
        &server,
        vec![
            sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
            sse(vec![ev_response_created("resp-2"), ev_completed("resp-2")]),
        ],
    )
    .await;

    let mut builder = test_codex().with_config({
        let base_url = format!("{}/v1", server.uri());
        move |config| {
            config.model_provider.base_url = Some(base_url);
            config.model_provider.wire_api = WireApi::Responses;
            config.model_provider.supports_websockets = true;
            config.model_provider.stream_max_retries = Some(2);
            config.model_provider.request_max_retries = Some(0);
        }
    });
    let test = builder.build(&server).await?;

    test.submit_turn("first").await?;
    test.submit_turn("second").await?;

    let requests = server.received_requests().await.unwrap_or_default();
    let websocket_attempts = requests
        .iter()
        .filter(|req| req.method == Method::GET && req.url.path().ends_with("/responses"))
        .count();
    let http_attempts = requests
        .iter()
        .filter(|req| req.method == Method::POST && req.url.path().ends_with("/responses"))
        .count();

    // WebSocket attempts all happen on the first turn:
    // 1 deferred request prewarm attempt (startup) + 3 stream attempts
    // (initial try + 2 retries) before fallback.
    // Fallback is sticky, so the second turn stays on HTTP and adds no websocket attempts.
    assert_eq!(websocket_attempts, 4);
    assert_eq!(http_attempts, 2);
    assert_eq!(response_mock.requests().len(), 2);

    Ok(())
}
