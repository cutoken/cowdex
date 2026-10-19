use super::*;
use codex_protocol::config_types::ServiceTier;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn fast_selection_does_not_write_global_config_or_leak_into_fresh_session() -> Result<()> {
    let (mut app, mut rx, _op_rx) = make_test_app_with_channels().await;
    let config_path = app.config.codex_home.join("config.toml");
    let saved_config = std::fs::read(&config_path).ok();
    let baseline_tier = app.config.service_tier.clone();
    app.chat_widget
        .set_service_tier(Some(ServiceTier::Fast.request_value().to_string()));
    let mut app_server = crate::start_embedded_app_server_for_picker(&app.config).await?;
    let mut tui = crate::tui::test_support::make_test_tui()?;
    while rx.try_recv().is_ok() {}
    app.handle_event(
        &mut tui,
        &mut app_server,
        AppEvent::PersistServiceTierSelection {
            service_tier: Some(ServiceTier::Fast.request_value().to_string()),
        },
    )
    .await?;
    assert_eq!(std::fs::read(&config_path).ok(), saved_config);
    assert_eq!(app.config.service_tier, baseline_tier);
    assert_eq!(app.fresh_session_config().service_tier, None);
    let cell = match rx.try_recv().expect("session tier notice") {
        AppEvent::InsertHistoryCell(cell) => cell,
        other => panic!("unexpected event {other:?}"),
    };
    insta::assert_snapshot!(
        "fast_selection_session_notice",
        lines_to_single_string(&cell.display_lines(/*width*/ 100))
    );
    app_server.shutdown().await?;
    Ok(())
}
