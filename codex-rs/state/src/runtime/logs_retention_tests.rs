use super::*;
use codex_utils_absolute_path::test_support::PathExt;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn global_log_budget_and_cleanup_reclaim_old_records() {
    let home = test_support::unique_temp_dir();
    let runtime = StateRuntime::init(
        crate::SqliteConfig::new_for_testing(home.abs()),
        "test".into(),
    )
    .await
    .unwrap();
    for i in 0..10 {
        runtime
            .insert_log(&LogEntry {
                ts: Utc::now().timestamp(),
                ts_nanos: 0,
                level: "WARN".into(),
                target: "retention".into(),
                feedback_log_body: Some("x".repeat(4 * 1024 * 1024)),
                message: None,
                thread_id: Some(format!("thread-{i}")),
                process_uuid: None,
                module_path: None,
                file: None,
                line: None,
            })
            .await
            .unwrap();
    }
    let (count, bytes): (i64, i64) =
        sqlx::query_as("SELECT COUNT(*), SUM(estimated_bytes) FROM logs")
            .fetch_one(runtime.logs_pool.as_ref())
            .await
            .unwrap();
    assert_eq!(count, 7);
    assert!(bytes < 32 * 1024 * 1024);
    sqlx::query("UPDATE logs SET ts = 0")
        .execute(runtime.logs_pool.as_ref())
        .await
        .unwrap();
    assert_eq!(runtime.cleanup_logs().await.unwrap(), 7);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM logs")
        .fetch_one(runtime.logs_pool.as_ref())
        .await
        .unwrap();
    assert_eq!(count, 0);
    let _ = tokio::fs::remove_dir_all(home).await;
}
