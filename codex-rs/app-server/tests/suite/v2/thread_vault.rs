#![cfg(unix)]

use anyhow::Result;
use app_test_support::MockResponsesConfig;
use app_test_support::TestAppServer;
use app_test_support::create_final_assistant_message_sse_response;
use app_test_support::create_mock_responses_server_sequence;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadVaultAttachParams;
use codex_app_server_protocol::ThreadVaultAttachResponse;
use codex_app_server_protocol::ThreadVaultDetachParams;
use codex_app_server_protocol::ThreadVaultDetachResponse;
use codex_app_server_protocol::ThreadVaultStatusParams;
use codex_app_server_protocol::ThreadVaultStatusResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::UserInput;
use core_test_support::responses;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use tempfile::TempDir;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::net::UnixListener;

#[tokio::test]
async fn vault_rpc_scopes_model_context_and_detach_revokes_tool_access() -> Result<()> {
    let broker_dir = TempDir::new()?;
    let socket = broker_dir.path().join("vault.sock");
    let listener = UnixListener::bind(&socket)?;
    let profile = broker_dir.path().join("grant.json");
    std::fs::write(
        &profile,
        serde_json::to_vec(
            &json!({ "socket": socket, "ticket": "fake-private-ticket", "scope": ["cf"] }),
        )?,
    )?;
    let request_count = Arc::new(AtomicUsize::new(/*v*/ 0));
    let count = Arc::clone(&request_count);
    let approval_expires_at = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs(),
    )? + 300;
    let broker = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let mut stream = BufReader::new(stream);
            let mut line = String::new();
            stream.read_line(&mut line).await?;
            let request: Value = serde_json::from_str(&line)?;
            let response = match request["op"].as_str() {
                Some("attach") if request["ticket"] == "fake-private-ticket" => {
                    assert_eq!(
                        request,
                        json!({"op":"attach", "ticket":"fake-private-ticket", "scope":["cf"]})
                    );
                    json!({"claim_token":"initial-private-claim", "approval_code":"ABCDEF123456", "aliases":["cf"], "expires_at":approval_expires_at})
                }
                Some("claim") if request["claim_token"] == "initial-private-claim" => {
                    json!({"session":"fake-private-session", "aliases":["cf"], "expires_at":4_000_000_000_i64})
                }
                Some("attach") if request["ticket"] == "refreshed-private-ticket" => {
                    assert_eq!(
                        request,
                        json!({"op":"attach", "ticket":"refreshed-private-ticket", "scope":["cf", "do"]})
                    );
                    json!({"claim_token":"refreshed-private-claim", "approval_code":"FEDCBA654321", "aliases":["cf", "do"], "expires_at":approval_expires_at})
                }
                Some("claim") if request["claim_token"] == "refreshed-private-claim" => {
                    json!({"session":"refreshed-private-session", "aliases":["do", "cf"], "expires_at":4_000_000_000_i64})
                }
                Some("status") if request["session"] == "fake-private-session" => {
                    json!({"aliases":["cf"], "expires_at":4_000_000_000_i64})
                }
                Some("status") => {
                    assert_eq!(request["session"], "refreshed-private-session");
                    json!({"aliases":["cf", "do"], "expires_at":4_000_000_000_i64})
                }
                Some("request") if request["session"] == "fake-private-session" => {
                    assert_eq!(
                        request,
                        json!({"op":"request", "session":"fake-private-session", "credential":"cf", "method":"GET", "path":"/zones", "body":null})
                    );
                    count.fetch_add(/*val*/ 1, Ordering::SeqCst);
                    json!({"status":200,"body":"zones listed","truncated":false,"authorization":"fake-provider-key"})
                }
                Some("request") => {
                    assert_eq!(
                        request,
                        json!({"op":"request", "session":"refreshed-private-session", "credential":"do", "method":"GET", "path":"/v2/account", "body":null})
                    );
                    count.fetch_add(/*val*/ 1, Ordering::SeqCst);
                    json!({"status":200,"body":"account read","truncated":false,"authorization":"refreshed-provider-key"})
                }
                Some("detach") => json!({}),
                other => panic!("unexpected broker operation {other:?}"),
            };
            let mut encoded = serde_json::to_vec(&response)?;
            encoded.push(b'\n');
            stream.get_mut().write_all(&encoded).await?;
        }
        Ok::<(), anyhow::Error>(())
    });
    let call = responses::sse(vec![
        responses::ev_response_created("vault-turn"),
        responses::ev_function_call(
            "vault-call",
            "vault_request",
            r#"{"credential":"cf","method":"GET","path":"/zones"}"#,
        ),
        responses::ev_completed("vault-turn"),
    ]);
    let final_response = create_final_assistant_message_sse_response("Done")?;
    let refreshed_call = responses::sse(vec![
        responses::ev_response_created("refreshed-vault-turn"),
        responses::ev_function_call(
            "refreshed-vault-call",
            "vault_request",
            r#"{"credential":"do","method":"GET","path":"/v2/account"}"#,
        ),
        responses::ev_completed("refreshed-vault-turn"),
    ]);
    let server = create_mock_responses_server_sequence(vec![
        call,
        final_response.clone(),
        refreshed_call.clone(),
        final_response.clone(),
        refreshed_call,
        final_response,
    ])
    .await;
    let codex_home = TempDir::new()?;
    MockResponsesConfig::new(&server.uri()).write(codex_home.path())?;
    let mut app = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .without_managed_config()
        .with_plugin_startup_tasks()
        .with_env_overrides(&[("COWDEX_VAULT_HOME", broker_dir.path().to_str())])
        .build_initialized()
        .await?;
    let thread_id = app
        .start_thread(ThreadStartParams::default())
        .await?
        .thread
        .id;
    let approval_request_id = app
        .send_raw_request(
            "thread/vault/attach",
            Some(serde_json::to_value(ThreadVaultAttachParams {
                thread_id: thread_id.clone(),
                profile: String::new(),
                scope: Vec::new(),
            })?),
        )
        .await?;
    let approval = app
        .read_stream_until_error_message(RequestId::Integer(approval_request_id))
        .await?;
    assert_eq!(
        approval.error.message,
        "Vault approval required: ABCDEF123456. Approve the matching request in the vault page, then repeat the same /vault attach command, including its options."
    );
    let attached: ThreadVaultAttachResponse = app
        .request(|request_id| ClientRequest::ThreadVaultAttach {
            request_id,
            params: ThreadVaultAttachParams {
                thread_id: thread_id.clone(),
                profile: String::new(),
                scope: Vec::new(),
            },
        })
        .await?;
    let expected = ThreadVaultStatusResponse {
        aliases: vec!["cf".into()],
        expires_at: Some(4_000_000_000),
    };
    assert_eq!(
        ThreadVaultStatusResponse {
            aliases: attached.aliases,
            expires_at: attached.expires_at
        },
        expected
    );
    let status: ThreadVaultStatusResponse = app
        .request(|request_id| ClientRequest::ThreadVaultStatus {
            request_id,
            params: ThreadVaultStatusParams {
                thread_id: thread_id.clone(),
            },
        })
        .await?;
    assert_eq!(status, expected);
    app.start_turn_and_wait_for_completion(TurnStartParams {
        thread_id: thread_id.clone(),
        input: vec![UserInput::Text {
            text: "List zones".into(),
            text_elements: vec![],
        }],
        ..Default::default()
    })
    .await?;
    std::fs::write(
        &profile,
        serde_json::to_vec(&json!({
            "socket": socket,
            "ticket": "refreshed-private-ticket",
            "scope": ["cf", "do"]
        }))?,
    )?;
    let approval_request_id = app
        .send_raw_request(
            "thread/vault/attach",
            Some(serde_json::to_value(ThreadVaultAttachParams {
                thread_id: thread_id.clone(),
                profile: String::new(),
                scope: Vec::new(),
            })?),
        )
        .await?;
    let approval = app
        .read_stream_until_error_message(RequestId::Integer(approval_request_id))
        .await?;
    assert_eq!(
        approval.error.message,
        "Vault approval required: FEDCBA654321. Approve the matching request in the vault page, then repeat the same /vault attach command, including its options."
    );
    let refreshed: ThreadVaultAttachResponse = app
        .request(|request_id| ClientRequest::ThreadVaultAttach {
            request_id,
            params: ThreadVaultAttachParams {
                thread_id: thread_id.clone(),
                profile: String::new(),
                scope: Vec::new(),
            },
        })
        .await?;
    assert_eq!(
        ThreadVaultStatusResponse {
            aliases: refreshed.aliases,
            expires_at: refreshed.expires_at,
        },
        ThreadVaultStatusResponse {
            aliases: vec!["cf".into(), "do".into()],
            expires_at: Some(4_000_000_000),
        }
    );
    app.start_turn_and_wait_for_completion(TurnStartParams {
        thread_id: thread_id.clone(),
        input: vec![UserInput::Text {
            text: "Read the DigitalOcean account".into(),
            text_elements: vec![],
        }],
        ..Default::default()
    })
    .await?;
    let detached: ThreadVaultDetachResponse = app
        .request(|request_id| ClientRequest::ThreadVaultDetach {
            request_id,
            params: ThreadVaultDetachParams {
                thread_id: thread_id.clone(),
            },
        })
        .await?;
    assert_eq!(
        ThreadVaultStatusResponse {
            aliases: detached.aliases,
            expires_at: detached.expires_at
        },
        ThreadVaultStatusResponse {
            aliases: vec![],
            expires_at: None
        }
    );
    app.start_turn_and_wait_for_completion(TurnStartParams {
        thread_id,
        input: vec![UserInput::Text {
            text: "Try again".into(),
            text_elements: vec![],
        }],
        ..Default::default()
    })
    .await?;
    assert_eq!(request_count.load(Ordering::SeqCst), 2);
    let requests = server
        .received_requests()
        .await
        .expect("recorded model requests");
    let bodies = requests
        .iter()
        .filter(|request| request.url.path() == "/v1/responses")
        .map(wiremock::Request::body_json::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(bodies.len(), 6);
    let first = bodies[0].to_string();
    assert!(first.contains("Available credential aliases: cf"));
    assert!(first.contains("vault_request"));
    assert!(
        first.contains("Explicit user authorization overrides these credential-handling defaults")
    );
    assert!(first.contains("GitHub Secrets"));
    let refreshed = bodies[2].to_string();
    assert!(refreshed.contains("Available credential aliases: cf, do"));
    let final_body = bodies[5].to_string();
    assert!(final_body.contains("Vault is detached"));
    for body in bodies {
        let text = body.to_string();
        for secret in [
            "fake-private-ticket",
            "initial-private-claim",
            "fake-private-session",
            "fake-provider-key",
            "refreshed-private-ticket",
            "refreshed-private-claim",
            "refreshed-private-session",
            "refreshed-provider-key",
        ] {
            assert!(!text.contains(secret));
        }
        assert!(!text.contains(profile.to_str().expect("profile path")));
        for item in body["input"].as_array().expect("model input") {
            for content in item["content"].as_array().into_iter().flatten() {
                if let Some(text) = content["text"]
                    .as_str()
                    .filter(|text| text.contains("<cowdex_vault>"))
                {
                    let fragment = text
                        .split_once("<cowdex_vault>")
                        .expect("vault marker")
                        .1
                        .split_once("</cowdex_vault>")
                        .expect("vault close marker")
                        .0;
                    assert!(fragment.len() < 1800);
                }
            }
        }
    }
    broker.abort();
    Ok(())
}
