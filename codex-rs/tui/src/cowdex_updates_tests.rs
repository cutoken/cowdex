use super::*;
use crate::legacy_core::config::ConfigBuilder;
use pretty_assertions::assert_eq;
use serde_json::json;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

#[tokio::test]
async fn release_refresh_preserves_dismissal_and_uses_a_separate_cache() -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let config = ConfigBuilder::default()
        .codex_home(home.path().to_path_buf())
        .build()
        .await?;
    let upstream_cache = home.path().join("version.json");
    std::fs::write(&upstream_cache, "regular Codex cache")?;
    dismiss_version(&config, "999.0.0-r1").await?;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/releases/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"tag_name":"cowdex-v999.0.0-r1"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let cache = version_filepath(&config);
    check_for_update(
        &cache,
        config.http_client_factory(),
        &format!("{}/releases/latest", server.uri()),
    )
    .await?;
    assert_eq!(get_upgrade_version_for_popup(&config), None);
    assert_eq!(get_upgrade_version(&config), Some("999.0.0-r1".to_string()));
    let mut info = read_version_info(&cache)?;
    info.latest_version = "999.0.0-r2".to_string();
    std::fs::write(&cache, serde_json::to_vec(&info)?)?;
    assert_eq!(
        get_upgrade_version_for_popup(&config),
        Some("999.0.0-r2".to_string())
    );
    assert_eq!(
        std::fs::read_to_string(upstream_cache)?,
        "regular Codex cache"
    );
    Ok(())
}
