//! Bounded, rebuildable history projections. Durable rollout files are never removed.

use codex_protocol::ThreadId;
use sqlx::SqlitePool;

use super::LocalThreadStore;
use super::rollout_lineage::RolloutLineage;
use crate::ThreadStoreError;
use crate::ThreadStoreResult;

pub(super) const CACHE_LIMIT_BYTES: i64 = 2 * 1024 * 1024 * 1024;
const READ_GRACE_MS: i64 = 60_000;

pub(super) async fn initialize(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    // Keep fork-specific cache metadata outside upstream's migration history.
    sqlx::query("CREATE TABLE IF NOT EXISTS cowdex_history_cache (thread_id TEXT PRIMARY KEY, last_used_ms INTEGER NOT NULL)")
        .execute(pool).await?;
    sqlx::query(
        "INSERT OR IGNORE INTO cowdex_history_cache
        SELECT p.thread_id, COALESCE(MAX(COALESCE(t.completed_at, t.started_at)), 0)*1000
        FROM thread_history_projection_state p LEFT JOIN thread_turns t ON t.thread_id = p.thread_id
        WHERE NOT EXISTS (SELECT 1 FROM cowdex_history_cache c WHERE c.thread_id = p.thread_id)
        GROUP BY p.thread_id",
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn touch(store: &LocalThreadStore, ids: &[ThreadId]) -> ThreadStoreResult<()> {
    let pool = store.thread_history_db().await?;
    for id in ids {
        sqlx::query("INSERT INTO cowdex_history_cache VALUES (?, ?) ON CONFLICT(thread_id) DO UPDATE SET last_used_ms = excluded.last_used_ms")
            .bind(id.to_string()).bind(chrono::Utc::now().timestamp_millis())
            .execute(pool).await.map_err(cache_error)?;
    }
    Ok(())
}

pub(super) async fn ensure_lineage(
    store: &LocalThreadStore,
    lineage: &RolloutLineage,
) -> ThreadStoreResult<()> {
    let ids: Vec<_> = lineage
        .segments()
        .iter()
        .map(|segment| segment.rollout_id)
        .collect();
    // Protect every ancestor before rebuilding any segment of a fork.
    touch(store, &ids).await?;
    for segment in lineage.segments() {
        if super::thread_history::projection_state(store, segment.rollout_id)
            .await?
            .is_some()
        {
            continue;
        }
        let path = codex_rollout::materialize_rollout_for_reference(&segment.rollout_path)
            .await
            .map_err(cache_error)?;
        super::thread_history_materialization::materialize_to_sqlite(
            store,
            segment.rollout_id,
            &path,
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn trim(store: &LocalThreadStore, max_bytes: i64) -> ThreadStoreResult<()> {
    let pool = store.thread_history_db().await?;
    let (used, free): (i64, i64) = sqlx::query_as(
        "SELECT (p.page_count-f.freelist_count)*s.page_size, f.freelist_count
         FROM pragma_page_count p, pragma_freelist_count f, pragma_page_size s",
    )
    .fetch_one(pool)
    .await
    .map_err(cache_error)?;
    if used <= max_bytes && free == 0 {
        return Ok(());
    }
    loop {
        let mut tx = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(cache_error)?;
        let pages: i64 = sqlx::query_scalar("PRAGMA page_count")
            .fetch_one(&mut *tx)
            .await
            .map_err(cache_error)?;
        let free: i64 = sqlx::query_scalar("PRAGMA freelist_count")
            .fetch_one(&mut *tx)
            .await
            .map_err(cache_error)?;
        let page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
            .fetch_one(&mut *tx)
            .await
            .map_err(cache_error)?;
        if (pages - free).saturating_mul(page_size) <= max_bytes {
            tx.commit().await.map_err(cache_error)?;
            break;
        }
        // Never evict an unfinished turn or a projection currently being read/rebuilt.
        let victims: Vec<String> = sqlx::query_scalar(
            "SELECT c.thread_id FROM cowdex_history_cache c
             JOIN thread_history_projection_state p ON p.thread_id = c.thread_id
             WHERE c.last_used_ms < ? AND NOT EXISTS (
                 SELECT 1 FROM thread_turns t WHERE t.thread_id = c.thread_id
                 AND t.status = 'inProgress')
             ORDER BY c.last_used_ms, c.thread_id LIMIT 32",
        )
        .bind(chrono::Utc::now().timestamp_millis() - READ_GRACE_MS)
        .fetch_all(&mut *tx)
        .await
        .map_err(cache_error)?;
        if victims.is_empty() {
            tx.commit().await.map_err(cache_error)?;
            break;
        }
        for id in victims {
            for statement in [
                "DELETE FROM thread_items WHERE thread_id = ?",
                "DELETE FROM thread_turns WHERE thread_id = ?",
                "DELETE FROM thread_history_projection_state WHERE thread_id = ?",
                "DELETE FROM cowdex_history_cache WHERE thread_id = ?",
            ] {
                sqlx::query(statement)
                    .bind(&id)
                    .execute(&mut *tx)
                    .await
                    .map_err(cache_error)?;
            }
        }
        tx.commit().await.map_err(cache_error)?;
    }
    // Incremental auto-vacuum is already enabled. Consume all pragma rows to actually free
    // the requested pages; checkpoint between small batches so the WAL stays bounded too.
    loop {
        let free: i64 = sqlx::query_scalar("PRAGMA freelist_count")
            .fetch_one(pool)
            .await
            .map_err(cache_error)?;
        if free == 0 {
            break;
        }
        sqlx::query("PRAGMA incremental_vacuum(1024)")
            .fetch_all(pool)
            .await
            .map_err(cache_error)?;
        sqlx::query("PRAGMA wal_checkpoint(PASSIVE)")
            .fetch_all(pool)
            .await
            .map_err(cache_error)?;
        let remaining: i64 = sqlx::query_scalar("PRAGMA freelist_count")
            .fetch_one(pool)
            .await
            .map_err(cache_error)?;
        if remaining >= free {
            break;
        }
    }
    Ok(())
}

fn cache_error(error: impl std::fmt::Display) -> ThreadStoreError {
    ThreadStoreError::Internal {
        message: format!("thread history cache: {error}"),
    }
}
