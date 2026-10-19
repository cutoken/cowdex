use super::*;
use pretty_assertions::assert_eq;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn broker_scope_inheritance_detach_and_fresh_runtime() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("broker.sock");
    let profile = directory.path().join("grant.json");
    std::fs::write(
        &profile,
        serde_json::to_vec(&json!({"socket":socket,"ticket":"one-use"})).unwrap(),
    )
    .unwrap();
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let broker = tokio::spawn(async move {
        for expected in ["attach", "request", "detach"] {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = tokio::io::BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["op"], expected);
            let response = match expected {
                "attach" => {
                    assert_eq!(
                        request,
                        json!({"op":"attach", "ticket":"one-use", "scope":["cloudflare-dev"]})
                    );
                    json!({"session":"hidden-session", "aliases":["cloudflare-dev"], "expires_at":4102444800_i64})
                }
                "request" => {
                    assert_eq!(
                        request,
                        json!({"op":"request", "session":"hidden-session", "credential":"cloudflare-dev", "method":"GET", "path":"/client/v4/zones", "body":null})
                    );
                    json!({"status":200,"body":"ok","truncated":false,"session":"must-not-leak"})
                }
                "detach" => json!({}),
                _ => unreachable!(),
            };
            reader
                .get_mut()
                .write_all(format!("{response}\n").as_bytes())
                .await
                .unwrap();
        }
    });
    let service = VaultService::new();
    let root = ThreadId::new();
    let child = ThreadId::new();
    let other = ThreadId::new();
    service.register(root, /*parent*/ None).await;
    service.register(child, Some(root)).await;
    service.register(other, /*parent*/ None).await;
    assert_eq!(
        service
            .attach(root, profile, vec!["cloudflare-dev".into()])
            .await
            .unwrap(),
        VaultStatus {
            aliases: vec!["cloudflare-dev".into()],
            expires_at: Some(4102444800)
        }
    );
    let request = || Request {
        credential: "cloudflare-dev".into(),
        method: "GET".into(),
        path: "/client/v4/zones".into(),
        body: None,
    };
    assert!(service.request(other, request()).await.is_err());
    let mut wrong = request();
    wrong.credential = "other".into();
    assert!(service.request(child, wrong).await.is_err());
    assert_eq!(
        service.request(child, request()).await.unwrap(),
        json!({"status":200,"body":"ok","truncated":false})
    );
    assert_eq!(service.detach(root).await.unwrap(), VaultStatus::default());
    assert!(service.request(child, request()).await.is_err());
    let resumed = VaultService::new();
    resumed.register(root, /*parent*/ None).await;
    assert_eq!(resumed.status(root).await.unwrap(), VaultStatus::default());
    broker.await.unwrap();
}

#[tokio::test]
async fn mismatched_broker_scope_is_revoked_without_attaching() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("broker.sock");
    let profile = directory.path().join("grant.json");
    std::fs::write(
        &profile,
        serde_json::to_vec(&json!({"socket":socket,"ticket":"one-use"})).unwrap(),
    )
    .unwrap();
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let broker = tokio::spawn(async move {
        for expected in ["attach", "detach"] {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = tokio::io::BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["op"], expected);
            let response = if expected == "attach" {
                json!({"session":"hidden-session", "aliases":["other"], "expires_at":4102444800_i64})
            } else {
                assert_eq!(request, json!({"op":"detach", "session":"hidden-session"}));
                json!({})
            };
            reader
                .get_mut()
                .write_all(format!("{response}\n").as_bytes())
                .await
                .unwrap();
        }
    });
    let service = VaultService::new();
    let root = ThreadId::new();
    service.register(root, /*parent*/ None).await;
    assert_eq!(
        service
            .attach(root, profile, vec!["cloudflare-dev".into()])
            .await,
        Err("Vault grant did not match the requested scope".into())
    );
    assert_eq!(service.status(root).await.unwrap(), VaultStatus::default());
    broker.await.unwrap();
}

#[tokio::test]
async fn reattach_replaces_the_grant_even_when_old_detach_fails() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("broker.sock");
    let profile = directory.path().join("grant.json");
    std::fs::write(
        &profile,
        serde_json::to_vec(&json!({"socket":socket,"ticket":"first"})).unwrap(),
    )
    .unwrap();
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let broker = tokio::spawn(async move {
        for expected in ["attach", "request", "attach", "detach", "request", "detach"] {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = tokio::io::BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["op"], expected);
            let response = match expected {
                "attach" if request["ticket"] == "first" => {
                    assert_eq!(request["scope"], json!(["cloudflare-dev"]));
                    json!({"session":"first-session","aliases":["cloudflare-dev"],"expires_at":4102444800_i64})
                }
                "attach" => {
                    assert_eq!(request["ticket"], "second");
                    assert_eq!(
                        request["scope"],
                        json!(["cloudflare-dev", "digitalocean-dev"])
                    );
                    json!({"session":"second-session","aliases":["digitalocean-dev","cloudflare-dev"],"expires_at":4102444800_i64})
                }
                "request" if request["session"] == "first-session" => {
                    assert_eq!(request["credential"], "cloudflare-dev");
                    json!({"status":200,"body":"first","truncated":false})
                }
                "request" => {
                    assert_eq!(request["session"], "second-session");
                    assert_eq!(request["credential"], "digitalocean-dev");
                    json!({"status":200,"body":"second","truncated":false})
                }
                "detach" if request["session"] == "first-session" => {
                    json!({"error":"already expired"})
                }
                "detach" => {
                    assert_eq!(request["session"], "second-session");
                    json!({})
                }
                _ => unreachable!(),
            };
            reader
                .get_mut()
                .write_all(format!("{response}\n").as_bytes())
                .await
                .unwrap();
        }
    });
    let service = VaultService::new();
    let root = ThreadId::new();
    let child = ThreadId::new();
    service.register(root, /*parent*/ None).await;
    service.register(child, Some(root)).await;
    service
        .attach(root, profile.clone(), vec!["cloudflare-dev".into()])
        .await
        .unwrap();
    assert_eq!(
        service
            .request(
                child,
                Request {
                    credential: "cloudflare-dev".into(),
                    method: "GET".into(),
                    path: "/client/v4/zones".into(),
                    body: None,
                },
            )
            .await
            .unwrap(),
        json!({"status":200,"body":"first","truncated":false})
    );
    std::fs::write(
        &profile,
        serde_json::to_vec(&json!({"socket":socket,"ticket":"second"})).unwrap(),
    )
    .unwrap();
    assert_eq!(
        service
            .attach(
                root,
                profile,
                vec!["digitalocean-dev".into(), "cloudflare-dev".into()],
            )
            .await
            .unwrap(),
        VaultStatus {
            aliases: vec!["cloudflare-dev".into(), "digitalocean-dev".into()],
            expires_at: Some(4102444800),
        }
    );
    assert_eq!(
        service
            .request(
                child,
                Request {
                    credential: "digitalocean-dev".into(),
                    method: "GET".into(),
                    path: "/v2/account".into(),
                    body: None,
                },
            )
            .await
            .unwrap(),
        json!({"status":200,"body":"second","truncated":false})
    );
    service.detach(root).await.unwrap();
    broker.await.unwrap();
}

#[tokio::test]
async fn failed_reattach_preserves_the_existing_grant() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("broker.sock");
    let profile = directory.path().join("grant.json");
    std::fs::write(
        &profile,
        serde_json::to_vec(&json!({"socket":socket,"ticket":"first"})).unwrap(),
    )
    .unwrap();
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let broker = tokio::spawn(async move {
        for expected in ["attach", "attach", "detach", "request", "detach"] {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = tokio::io::BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["op"], expected);
            let response = match expected {
                "attach" if request["ticket"] == "first" => {
                    json!({"session":"first-session","aliases":["cloudflare-dev"],"expires_at":4102444800_i64})
                }
                "attach" => {
                    assert_eq!(request["ticket"], "second");
                    json!({"session":"rejected-session","aliases":["other"],"expires_at":4102444800_i64})
                }
                "request" => {
                    assert_eq!(request["session"], "first-session");
                    assert_eq!(request["credential"], "cloudflare-dev");
                    json!({"status":200,"body":"still-live","truncated":false})
                }
                "detach" if request["session"] == "rejected-session" => json!({}),
                "detach" => {
                    assert_eq!(request["session"], "first-session");
                    json!({})
                }
                _ => unreachable!(),
            };
            reader
                .get_mut()
                .write_all(format!("{response}\n").as_bytes())
                .await
                .unwrap();
        }
    });
    let service = VaultService::new();
    let root = ThreadId::new();
    let child = ThreadId::new();
    service.register(root, /*parent*/ None).await;
    service.register(child, Some(root)).await;
    service
        .attach(root, profile.clone(), vec!["cloudflare-dev".into()])
        .await
        .unwrap();
    std::fs::write(
        &profile,
        serde_json::to_vec(&json!({"socket":socket,"ticket":"second"})).unwrap(),
    )
    .unwrap();
    assert_eq!(
        service
            .attach(root, profile, vec!["digitalocean-dev".into()])
            .await,
        Err("Vault grant did not match the requested scope".into())
    );
    assert_eq!(
        service
            .request(
                child,
                Request {
                    credential: "cloudflare-dev".into(),
                    method: "GET".into(),
                    path: "/client/v4/zones".into(),
                    body: None,
                },
            )
            .await
            .unwrap(),
        json!({"status":200,"body":"still-live","truncated":false})
    );
    service.detach(root).await.unwrap();
    broker.await.unwrap();
}

#[tokio::test]
async fn approval_pending_preserves_grant_until_claim_succeeds() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("broker.sock");
    let profile = directory.path().join("grant.json");
    std::fs::write(
        &profile,
        serde_json::to_vec(&json!({"socket":socket,"ticket":"first"})).unwrap(),
    )
    .unwrap();
    let approval_expires_at = unix_time().unwrap() + 300;
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let broker = tokio::spawn(async move {
        for step in 0..12 {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = tokio::io::BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let response = match step {
                0 => {
                    assert_eq!(request["op"], "attach");
                    json!({"session":"first-session","aliases":["cf"],"expires_at":4102444800_i64})
                }
                1 => {
                    assert_eq!(request["op"], "attach");
                    json!({"claim_token":"private-claim","approval_code":"ABCDEF123456","aliases":["cf","do"],"expires_at":approval_expires_at})
                }
                2 => {
                    assert_eq!(request["session"], "first-session");
                    json!({"status":200,"body":"old grant remains live","truncated":false})
                }
                3 => {
                    assert_eq!(request, json!({"op":"claim","claim_token":"private-claim"}));
                    json!({"claim_token":"private-claim","approval_code":"ABCDEF123456","aliases":["cf","do"],"expires_at":approval_expires_at})
                }
                4 => {
                    assert_eq!(request, json!({"op":"claim","claim_token":"private-claim"}));
                    json!({"session":"second-session","aliases":["do","cf"],"expires_at":4102444800_i64})
                }
                5 => {
                    assert_eq!(request, json!({"op":"detach","session":"first-session"}));
                    json!({})
                }
                6 => {
                    assert_eq!(request["session"], "second-session");
                    json!({"status":200,"body":"new grant is live","truncated":false})
                }
                7 => {
                    assert_eq!(request["scope"], json!(["github"]));
                    json!({"claim_token":"third-claim","approval_code":"333333333333","aliases":["github"],"expires_at":approval_expires_at})
                }
                8 => {
                    assert_eq!(request, json!({"op":"cancel","claim_token":"third-claim"}));
                    json!({})
                }
                9 => {
                    assert_eq!(request["scope"], json!(["mailgun"]));
                    json!({"claim_token":"fourth-claim","approval_code":"444444444444","aliases":["mailgun"],"expires_at":approval_expires_at})
                }
                10 => {
                    assert_eq!(request, json!({"op":"cancel","claim_token":"fourth-claim"}));
                    json!({})
                }
                11 => {
                    assert_eq!(request, json!({"op":"detach","session":"second-session"}));
                    json!({})
                }
                _ => unreachable!(),
            };
            reader
                .get_mut()
                .write_all(format!("{response}\n").as_bytes())
                .await
                .unwrap();
        }
    });
    let service = VaultService::new();
    let root = ThreadId::new();
    let child = ThreadId::new();
    service.register(root, /*parent*/ None).await;
    service.register(child, Some(root)).await;
    service
        .attach(root, profile.clone(), vec!["cf".into()])
        .await
        .unwrap();
    std::fs::write(
        &profile,
        serde_json::to_vec(&json!({"socket":socket,"ticket":"second"})).unwrap(),
    )
    .unwrap();
    let approval_message = "Vault approval required: ABCDEF123456. Approve the matching request in the vault page, then repeat the same /vault attach command, including its options.";
    assert_eq!(
        service
            .attach(root, profile.clone(), vec!["do".into(), "cf".into()])
            .await,
        Err(approval_message.into())
    );
    assert_eq!(
        service
            .request(
                child,
                Request {
                    credential: "cf".into(),
                    method: "GET".into(),
                    path: "/zones".into(),
                    body: None,
                },
            )
            .await
            .unwrap(),
        json!({"status":200,"body":"old grant remains live","truncated":false})
    );
    assert_eq!(
        service
            .attach(root, profile.clone(), vec!["cf".into(), "do".into()])
            .await,
        Err(approval_message.into())
    );
    assert_eq!(
        service
            .attach(root, profile.clone(), vec!["cf".into(), "do".into()])
            .await
            .unwrap(),
        VaultStatus {
            aliases: vec!["cf".into(), "do".into()],
            expires_at: Some(4102444800),
        }
    );
    assert_eq!(
        service
            .request(
                child,
                Request {
                    credential: "do".into(),
                    method: "GET".into(),
                    path: "/v2/account".into(),
                    body: None,
                },
            )
            .await
            .unwrap(),
        json!({"status":200,"body":"new grant is live","truncated":false})
    );
    assert!(
        service
            .attach(root, profile.clone(), vec!["github".into()])
            .await
            .is_err()
    );
    assert!(
        service
            .attach(root, profile, vec!["mailgun".into()])
            .await
            .is_err()
    );
    service.detach(root).await.unwrap();
    broker.await.unwrap();
}
