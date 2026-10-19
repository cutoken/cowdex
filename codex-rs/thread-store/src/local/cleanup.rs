use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;
use std::time::SystemTime;

use codex_protocol::ThreadId;
use codex_rollout::RolloutReferenceIndex;

use super::LocalThreadStore;
use crate::DeleteThreadsParams;
use crate::ThreadStore;
use crate::ThreadStoreError;
use crate::ThreadStoreResult;

/// Storage reclaimed by removing expired subagent sessions.
#[derive(Debug, Default)]
pub struct SubagentCleanupReport {
    pub removed_sessions: usize,
    pub reclaimed_bytes: u64,
    pub kept_referenced: usize,
}

impl LocalThreadStore {
    /// Remove subagent sessions unused for seven days, retaining live writers and fork sources.
    pub async fn cleanup_stale_subagents(&self) -> ThreadStoreResult<SubagentCleanupReport> {
        let cutoff = SystemTime::now() - Duration::from_secs(7 * 24 * 60 * 60);
        let index = RolloutReferenceIndex::scan(&self.config.codex_home)
            .await
            .map_err(cleanup_error)?;
        let mut ages = HashMap::new();
        for (id, path) in index.subagent_rollouts() {
            let modified = tokio::fs::metadata(path)
                .await
                .map_err(cleanup_error)?
                .modified()
                .map_err(cleanup_error)?;
            ages.entry(id)
                .and_modify(|last: &mut SystemTime| *last = (*last).max(modified))
                .or_insert(modified);
        }
        let mut candidates = HashSet::new();
        for (id, modified) in ages {
            if modified >= cutoff {
                continue;
            }
            if let Some(state) = self.state_db.as_ref()
                && let Some(metadata) = state.get_thread(id).await.map_err(cleanup_error)?
                && SystemTime::from(metadata.updated_at) >= cutoff
            {
                continue;
            }
            // The actual delete re-acquires these cross-process writer locks.
            if !self.live_recorders.lock().await.contains_key(&id)
                && self.writer_lock_coordinator.acquire(id).is_ok()
            {
                candidates.insert(id);
            }
        }
        let initial = candidates.len();
        loop {
            let rollouts: HashSet<_> = candidates
                .iter()
                .flat_map(|id| index.rollouts_for_thread(*id).map(|(rollout, _)| rollout))
                .collect();
            let mut internal = HashMap::new();
            for id in &rollouts {
                if let Some(base) = index.history_base(*id)
                    && base.thread_id != *id
                    && rollouts.contains(&base.thread_id)
                {
                    *internal.entry(base.thread_id).or_insert(0usize) += 1;
                }
            }
            let before = candidates.len();
            candidates.retain(|id| {
                index.rollouts_for_thread(*id).all(|(rollout, _)| {
                    index.reference_count(rollout) <= internal.get(&rollout).copied().unwrap_or(0)
                })
            });
            if candidates.len() == before {
                break;
            }
        }
        let mut report = SubagentCleanupReport {
            kept_referenced: initial - candidates.len(),
            ..Default::default()
        };
        let mut removed_references = HashMap::new();
        // Delete leaves first in bounded batches: the existing delete operation holds one
        // descriptor per thread until the whole batch completes.
        while !candidates.is_empty() {
            let mut ids: Vec<ThreadId> = candidates
                .iter()
                .copied()
                .filter(|id| {
                    let owned: Vec<_> = index
                        .rollouts_for_thread(*id)
                        .map(|(rollout, _)| rollout)
                        .collect();
                    owned.iter().all(|rollout| {
                        let own_references = owned
                            .iter()
                            .filter(|source| {
                                **source != *rollout
                                    && index
                                        .history_base(**source)
                                        .is_some_and(|base| base.thread_id == *rollout)
                            })
                            .count();
                        index.reference_count(*rollout)
                            <= own_references
                                + removed_references.get(rollout).copied().unwrap_or(0)
                    })
                })
                .take(128)
                .collect();
            if ids.is_empty() {
                report.kept_referenced += candidates.len();
                break;
            }
            ids.sort_unstable_by_key(ToString::to_string);
            let mut bytes = 0;
            for id in &ids {
                for (_, path) in index.rollouts_for_thread(*id) {
                    bytes += tokio::fs::metadata(path)
                        .await
                        .map_err(cleanup_error)?
                        .len();
                }
            }
            self.delete_threads(DeleteThreadsParams {
                thread_ids: ids.clone(),
            })
            .await?;
            if let Some(state) = self.state_db.as_ref() {
                state
                    .delete_threads_strict(&ids)
                    .await
                    .map_err(cleanup_error)?;
            }
            for id in &ids {
                candidates.remove(id);
                for (rollout, _) in index.rollouts_for_thread(*id) {
                    if let Some(base) = index.history_base(rollout)
                        && base.thread_id != rollout
                    {
                        *removed_references.entry(base.thread_id).or_insert(0usize) += 1;
                    }
                }
            }
            report.removed_sessions += ids.len();
            report.reclaimed_bytes += bytes;
        }
        self.trim_history_cache().await?;
        Ok(report)
    }

    /// Evict old rebuildable projections and return their unused pages to the filesystem.
    pub async fn trim_history_cache(&self) -> ThreadStoreResult<()> {
        if self.state_db.is_some()
            && tokio::fs::try_exists(self.config.sqlite.thread_history_db_path())
                .await
                .map_err(cleanup_error)?
        {
            super::history_cache::trim(self, super::history_cache::CACHE_LIMIT_BYTES).await?;
        }
        Ok(())
    }
}

fn cleanup_error(error: impl std::fmt::Display) -> ThreadStoreError {
    ThreadStoreError::Internal {
        message: format!("storage cleanup: {error}"),
    }
}
