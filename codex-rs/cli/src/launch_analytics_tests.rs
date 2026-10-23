use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

#[tokio::test]
async fn launch_is_daily_and_keeps_one_anonymous_identity_across_updates() -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/launch"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let endpoint = format!("{}/launch", server.uri());
    send_daily_launch(home.path(), &endpoint, /*now*/ 86_400, "1.0.0").await?;
    send_daily_launch(home.path(), &endpoint, /*now*/ 86_401, "1.1.0").await?;
    send_daily_launch(home.path(), &endpoint, /*now*/ 172_800, "1.1.0").await?;
    let requests = server.received_requests().await.expect("request recording");
    let first: serde_json::Value = serde_json::from_slice(&requests[0].body)?;
    let id = first["installation_id"].as_str().expect("anonymous id");
    assert!(ThreadId::from_string(id).is_ok());
    assert_eq!(
        requests
            .iter()
            .map(|request| serde_json::from_slice::<serde_json::Value>(&request.body).unwrap())
            .collect::<Vec<_>>(),
        vec![
            json!({"event":"launch", "installation_id":id, "version":"1.0.0", "os":std::env::consts::OS, "arch":std::env::consts::ARCH, "first_launch":true, "updated":false}),
            json!({"event":"launch", "installation_id":id, "version":"1.1.0", "os":std::env::consts::OS, "arch":std::env::consts::ARCH, "first_launch":false, "updated":true}),
        ]
    );
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(home.path().join("cowdex-analytics.json"))?)?;
    assert_eq!(
        state,
        json!({"installation_id":id, "last_day":2, "version":"1.1.0"})
    );
    Ok(())
}

#[tokio::test]
async fn simultaneous_launches_send_once_and_failures_can_retry() -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let server = MockServer::start().await;
    let endpoint = format!("{}/launch", server.uri());
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        send_daily_launch(home.path(), &endpoint, /*now*/ 86_400, "1.0.0")
            .await
            .is_err()
    );
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(204).set_delay(Duration::from_millis(/*millis*/ 100)))
        .expect(1)
        .mount(&server)
        .await;
    let (first, second) = tokio::join!(
        send_daily_launch(home.path(), &endpoint, /*now*/ 86_400, "1.0.0"),
        send_daily_launch(home.path(), &endpoint, /*now*/ 86_400, "1.0.0"),
    );
    first?;
    second?;
    server.verify().await;
    Ok(())
}
