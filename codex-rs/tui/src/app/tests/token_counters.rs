use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn live_token_notifications_update_bottom_status_for_main_and_worker() -> Result<()> {
    let (mut app, _rx, _op_rx) = make_test_app_with_channels().await;
    app.chat_widget.setup_status_line(
        vec![crate::bottom_pane::StatusLineItem::ContextWindowSize],
        /*use_theme_colors*/ true,
    );
    let app_server =
        crate::start_embedded_app_server_for_picker(app.chat_widget.config_ref()).await?;
    for thread in [ThreadId::new(), ThreadId::new()] {
        for notification in [
            turn_started_notification(thread, "running-turn"),
            token_usage_notification(thread, "running-turn", Some(950_000)),
            token_usage_notification(thread, "running-turn", Some(950_000)),
        ] {
            app.handle_app_server_event(
                &app_server,
                codex_app_server_client::AppServerEvent::ServerNotification(Box::new(notification)),
            )
            .await;
        }
    }
    let expected = "T 20 | CI 2 | NI 6 | OUT_R 0 | OUT_T 10";
    assert_eq!(
        app.chat_widget.status_line_text().as_deref(),
        Some(expected)
    );
    insta::assert_snapshot!(
        "live_token_counters",
        app.chat_widget.status_line_text().unwrap()
    );
    app_server.shutdown().await?;
    Ok(())
}
