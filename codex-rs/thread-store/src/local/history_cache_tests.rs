use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn cache_budget_evicts_old_projections_preserves_active_turns_and_rebuilds_fork_history() {
    let home = TempDir::new().unwrap();
    let store = projection_store(home.path()).await;
    let source = ThreadId::new();
    create_paginated_thread(&store, source).await;
    store
        .append_items(AppendThreadItemsParams {
            thread_id: source,
            items: vec![
                turn_started("source"),
                completed_item(source, "source", large_message("source-result")),
                turn_completed("source"),
            ],
        })
        .await
        .unwrap();
    store.flush_thread(source).await.unwrap();
    let source_path = store.live_rollout_path(source).await.unwrap();
    let durable = fs::read(&source_path).unwrap();
    let base = prepare_paginated_fork(&store, source, ForkBoundary::Latest)
        .await
        .history_base;
    store.shutdown_thread(source).await.unwrap();
    let child = ThreadId::new();
    create_paginated_subagent_thread(
        &store, child, base, /*subagent_history_start_ordinal*/ None,
    )
    .await;
    store
        .append_items(AppendThreadItemsParams {
            thread_id: child,
            items: vec![
                turn_started("child"),
                completed_item(child, "child", large_message("child-result")),
                turn_completed("child"),
            ],
        })
        .await
        .unwrap();
    store.flush_thread(child).await.unwrap();
    store
        .persist_thread(child, PersistContext::Standard)
        .await
        .unwrap();
    let active = ThreadId::new();
    create_paginated_thread(&store, active).await;
    store
        .append_items(AppendThreadItemsParams {
            thread_id: active,
            items: vec![turn_started("active")],
        })
        .await
        .unwrap();
    store.flush_thread(active).await.unwrap();

    let params = ListTurnsParams {
        thread_id: child,
        include_archived: false,
        cursor: None,
        page_size: 10,
        sort_direction: SortDirection::Asc,
        items_view: StoredTurnItemsView::Summary,
    };
    let child_path = store.live_rollout_path(child).await.unwrap();
    let (items, _, _) = RolloutRecorder::load_rollout_items(&child_path)
        .await
        .unwrap();
    let metadata = codex_rollout::builder_from_items(&items, &child_path)
        .unwrap()
        .build("test-provider");
    store
        .state_db()
        .await
        .unwrap()
        .upsert_thread(&metadata)
        .await
        .unwrap();
    let before = serde_json::to_value(store.list_turns(params.clone()).await.unwrap()).unwrap();
    let pool = store.thread_history_db().await.unwrap();
    sqlx::query("UPDATE cowdex_history_cache SET last_used_ms = 0 WHERE thread_id IN (?, ?)")
        .bind(source.to_string())
        .bind(active.to_string())
        .execute(pool)
        .await
        .unwrap();
    crate::local::history_cache::trim(&store, /*max_bytes*/ 450_000)
        .await
        .unwrap();
    let remaining: Vec<String> = sqlx::query_scalar(
        "SELECT thread_id FROM thread_history_projection_state ORDER BY thread_id",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let mut expected = vec![child.to_string(), active.to_string()];
    expected.sort();
    assert_eq!(remaining, expected);
    let bytes: i64 =
        sqlx::query_scalar("SELECT page_count*page_size FROM pragma_page_count, pragma_page_size")
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(
        bytes <= 450_000,
        "physical cache size after vacuum: {bytes}"
    );
    assert_eq!(fs::read(&source_path).unwrap(), durable);

    compress_rollout(&source_path);
    let after = serde_json::to_value(store.list_turns(params).await.unwrap()).unwrap();
    assert_eq!(after, before);
    assert_eq!(fs::read(&source_path).unwrap(), durable);
}

fn large_message(id: &str) -> TurnItem {
    TurnItem::AgentMessage(AgentMessageItem {
        id: id.to_string(),
        content: vec![AgentMessageContent::Text {
            text: "data".repeat(64 * 1024),
        }],
        phase: Some(MessagePhase::FinalAnswer),
        memory_citation: None,
    })
}
