use super::AgentControl;
use crate::agent::AgentStatus;
use crate::codex_thread::CodexThread;
use crate::config::Config;
use crate::session::InputQueue;
use crate::thread_manager::ThreadManagerState;
use codex_protocol::ThreadId;
use codex_protocol::error::CodexErr;
use codex_protocol::error::CodexErrorDetails;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::protocol::MultiAgentVersion;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::TurnEnvironmentSelection;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use tracing::warn;

/// Result of unloading one exact idle multi-agent v2 child runtime.
#[derive(Debug, PartialEq, Eq)]
pub enum ThreadEvictionOutcome {
    Busy,
    Evicted,
    NotCurrent,
}

#[derive(Clone)]
pub(crate) struct EvictedAgentState {
    pub(crate) environments: Vec<TurnEnvironmentSelection>,
    pub(crate) input_queue: Arc<InputQueue>,
}

#[derive(Default)]
pub(super) struct V2Residency {
    state: Mutex<V2ResidencyState>,
    pub(super) reload_lock: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct V2ResidencyState {
    residents: VecDeque<ThreadId>,
    pending_slots: usize,
    evicted: HashMap<ThreadId, EvictedAgentState>,
}

pub(super) struct V2ResidencySlot {
    residency: Arc<V2Residency>,
    active: bool,
}

impl V2ResidencySlot {
    pub(super) fn commit(mut self, thread_id: ThreadId) {
        self.residency.commit_slot(thread_id);
        self.active = false;
    }
}

impl Drop for V2ResidencySlot {
    fn drop(&mut self) {
        if self.active {
            self.residency.release_pending_slot();
        }
    }
}

impl AgentControl {
    pub(crate) fn evicted_state(&self, thread_id: ThreadId) -> Option<EvictedAgentState> {
        self.v2_residency
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .evicted
            .get(&thread_id)
            .cloned()
    }

    pub(crate) async fn try_evict_v2_thread(
        &self,
        state: &Arc<ThreadManagerState>,
        thread: Arc<CodexThread>,
    ) -> CodexResult<ThreadEvictionOutcome> {
        self.v2_residency.try_evict_thread(state, thread).await
    }

    pub(super) async fn reserve_v2_residency_slot(
        &self,
        state: &Arc<ThreadManagerState>,
        _config: &Config,
        protected_thread_id: Option<ThreadId>,
    ) -> CodexResult<V2ResidencySlot> {
        let capacity = self.agent_execution_limiter.max_threads();
        Arc::clone(&self.v2_residency)
            .reserve_slot(state, capacity, protected_thread_id)
            .await
    }

    pub(super) async fn touch_loaded_v2_residency(
        &self,
        state: &Arc<ThreadManagerState>,
        thread_id: ThreadId,
    ) {
        if let Ok(thread) = state.get_thread(thread_id).await
            && is_resident_candidate(thread.as_ref())
        {
            self.v2_residency.touch(thread_id);
        }
    }

    pub(super) fn forget_v2_residency(&self, thread_id: ThreadId) {
        self.v2_residency.remove(thread_id);
        self.v2_residency
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .evicted
            .remove(&thread_id);
    }
}

impl V2Residency {
    async fn reserve_slot(
        self: Arc<Self>,
        manager: &Arc<ThreadManagerState>,
        capacity: usize,
        protected_thread_id: Option<ThreadId>,
    ) -> CodexResult<V2ResidencySlot> {
        loop {
            if self.try_reserve_pending_slot(capacity) {
                return Ok(V2ResidencySlot {
                    residency: self,
                    active: true,
                });
            }
            if !self
                .try_unload_one_resident(manager, protected_thread_id)
                .await
            {
                return Err(CodexErr::new(CodexErrorDetails::AgentLimitReached {
                    max_threads: capacity,
                }));
            }
        }
    }

    fn try_reserve_pending_slot(&self, capacity: usize) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.residents.len().saturating_add(state.pending_slots) >= capacity {
            return false;
        }
        state.pending_slots += 1;
        true
    }

    async fn try_unload_one_resident(
        self: &Arc<Self>,
        manager: &Arc<ThreadManagerState>,
        protected_thread_id: Option<ThreadId>,
    ) -> bool {
        let candidates_to_scan = self.resident_count();
        for _ in 0..candidates_to_scan {
            let Some(candidate_thread_id) = self.pop_lru_candidate(protected_thread_id) else {
                return false;
            };
            let Some(candidate_thread) = manager
                .get_thread(candidate_thread_id)
                .await
                .ok()
                .filter(|thread| is_resident_candidate(thread))
            else {
                self.remove(candidate_thread_id);
                continue;
            };
            match self.try_evict_thread(manager, candidate_thread).await {
                Ok(ThreadEvictionOutcome::Evicted | ThreadEvictionOutcome::NotCurrent) => {
                    return true;
                }
                Ok(ThreadEvictionOutcome::Busy) => {}
                Err(err) => warn!("failed to unload v2 resident {candidate_thread_id}: {err}"),
            }
            self.touch(candidate_thread_id);
        }
        false
    }

    async fn try_evict_thread(
        self: &Arc<Self>,
        manager: &Arc<ThreadManagerState>,
        thread: Arc<CodexThread>,
    ) -> CodexResult<ThreadEvictionOutcome> {
        let Ok(residency_guard) = Arc::clone(&thread.residency_gate).try_write_owned() else {
            return Ok(ThreadEvictionOutcome::Busy);
        };
        let thread_id = thread.session.thread_id;
        if !manager
            .get_thread(thread_id)
            .await
            .is_ok_and(|current| Arc::ptr_eq(&current, &thread))
        {
            return Ok(ThreadEvictionOutcome::NotCurrent);
        }
        if !is_resident_candidate(&thread) || !is_unloadable(&thread).await {
            return Ok(ThreadEvictionOutcome::Busy);
        }
        let manager = Arc::clone(manager);
        let residency = Arc::clone(self);
        // Once shutdown starts, cancellation must not drop the pin or lose queued mail.
        tokio::spawn(async move {
            let _residency_guard = residency_guard;
            thread.ensure_rollout_materialized().await;
            let saved = EvictedAgentState {
                environments: thread.environment_selections().await,
                input_queue: Arc::clone(&thread.session.input_queue),
            };
            thread.shutdown_and_wait().await?;
            residency
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .evicted
                .insert(thread_id, saved);
            if manager
                .remove_thread_if_matches(&thread_id, &thread)
                .await
                .is_none()
            {
                residency
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .evicted
                    .remove(&thread_id);
                return Ok(ThreadEvictionOutcome::NotCurrent);
            }
            residency.remove(thread_id);
            Ok(ThreadEvictionOutcome::Evicted)
        })
        .await?
    }

    fn resident_count(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .residents
            .len()
    }

    fn pop_lru_candidate(&self, protected_thread_id: Option<ThreadId>) -> Option<ThreadId> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let candidates_to_scan = state.residents.len();
        for _ in 0..candidates_to_scan {
            let candidate_thread_id = state.residents.pop_front()?;
            if Some(candidate_thread_id) == protected_thread_id {
                state.residents.push_back(candidate_thread_id);
                continue;
            }
            state.residents.push_back(candidate_thread_id);
            return Some(candidate_thread_id);
        }
        None
    }

    fn touch(&self, thread_id: ThreadId) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        touch_resident(&mut state.residents, thread_id);
    }

    fn remove(&self, thread_id: ThreadId) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .residents
            .retain(|resident_thread_id| *resident_thread_id != thread_id);
    }

    fn commit_slot(&self, thread_id: ThreadId) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.pending_slots = state.pending_slots.saturating_sub(1);
        state.evicted.remove(&thread_id);
        touch_resident(&mut state.residents, thread_id);
    }

    fn release_pending_slot(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.pending_slots = state.pending_slots.saturating_sub(1);
    }
}

fn touch_resident(residents: &mut VecDeque<ThreadId>, thread_id: ThreadId) {
    residents.retain(|resident_thread_id| *resident_thread_id != thread_id);
    residents.push_back(thread_id);
}

fn is_resident_candidate(thread: &CodexThread) -> bool {
    thread.multi_agent_version() == Some(MultiAgentVersion::V2)
        && is_v2_resident_session_source(&thread.session_source)
}

pub(super) fn is_v2_resident_session_source(session_source: &SessionSource) -> bool {
    matches!(session_source, SessionSource::SubAgent(_))
}

async fn is_unloadable(thread: &CodexThread) -> bool {
    matches!(
        thread.agent_status().await,
        AgentStatus::Completed(_) | AgentStatus::Errored(_) | AgentStatus::Interrupted
    ) && thread.session.active_turn.lock().await.is_none()
        && !thread.session.has_outstanding_durable_sleep()
}

#[cfg(test)]
#[path = "residency_tests.rs"]
mod tests;
