use super::SessionSettingsUpdate;
use super::tests::make_session_and_context;
use crate::compact::CompactedHistoryMetadata;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::BossModeSettings;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn compaction_preserves_live_boss_settings_over_stale_turn_context() {
    let (session, turn_context) = make_session_and_context().await;
    let mut stale_context = turn_context.to_turn_context_item();

    for enabled in [true, false] {
        let settings = BossModeSettings {
            enabled,
            model: "gpt-5.4".to_string(),
            reasoning_effort: ReasoningEffort::High,
        };
        session
            .update_settings(SessionSettingsUpdate {
                boss_mode: Some(settings.clone()),
                ..Default::default()
            })
            .await
            .expect("update boss settings while the old turn context remains active");
        let (window_number, window_ids) = session.advance_auto_compact_window().await;
        session
            .replace_compacted_history(
                Vec::new(),
                Some(stale_context.clone()),
                /*world_state_baseline*/ None,
                CompactedHistoryMetadata {
                    message: "Compacted summary".to_string(),
                    window_number,
                    window_ids,
                },
            )
            .await;

        let mut expected_context = stale_context.clone();
        expected_context.boss_mode = Some(settings);
        assert_eq!(
            session.reference_context_item().await,
            Some(expected_context.clone())
        );
        let history = session.clone_history().await;
        let items = history.raw_items().collect::<Vec<_>>();
        assert_eq!(items.len(), 1);
        let ResponseItem::Message { role, content, .. } = items[0] else {
            panic!("compaction must retain the latest boss guidance message");
        };
        assert_eq!(role, "developer");
        let [ContentItem::InputText { text }] = content.as_slice() else {
            panic!("expected boss guidance text");
        };
        let expected_mode = if enabled { "on" } else { "off" };
        assert!(text.contains(&format!("Boss mode is {expected_mode}.")));
        assert!(text.contains("<boss_mode>"));

        // The next toggle races with a turn that still carries the previous mode.
        stale_context = expected_context;
    }
}
