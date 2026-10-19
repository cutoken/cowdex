use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Weak;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use codex_core::StartIfIdleSubmission;
use codex_core::ThreadManager;
use codex_extension_api::ExtensionEventSink;
use codex_protocol::ThreadId;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::TimerStatusEvent;
use serde::Serialize;
use tokio::sync::Mutex;
use tokio::sync::Semaphore;
use tokio::sync::watch;
use tokio::time::Instant;
use uuid::Uuid;

pub(crate) const MAX_TIMERS_PER_THREAD: usize = 64;
pub(crate) const MAX_REMINDER_BYTES: usize = 800;
const MAX_REMINDER_BATCH_BYTES: usize = 1024;
pub(crate) const MAX_DELAY_MS: u64 = 365 * 24 * 60 * 60 * 1000;
pub(crate) const MIN_INTERVAL_MS: u64 = 60_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerInfo {
    pub id: String,
    pub reminder: String,
    pub next_fire_at: i64,
    pub interval_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimerStatus {
    pub active_count: u32,
    pub next_fire_at: Option<i64>,
}

#[derive(Default)]
pub struct TimerService {
    runtimes: Mutex<HashMap<ThreadId, Vec<Weak<TimerRuntime>>>>,
}

impl TimerService {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn register(&self, thread_id: ThreadId, runtime: &Arc<TimerRuntime>) {
        let mut runtimes = self.runtimes.lock().await;
        let entries = runtimes.entry(thread_id).or_default();
        entries.retain(|entry| entry.strong_count() > 0);
        let runtime = Arc::downgrade(runtime);
        if !entries.iter().any(|entry| Weak::ptr_eq(entry, &runtime)) {
            entries.push(runtime);
        }
    }

    pub(crate) async fn unregister(&self, thread_id: ThreadId, runtime: &Arc<TimerRuntime>) {
        let mut runtimes = self.runtimes.lock().await;
        let Some(entries) = runtimes.get_mut(&thread_id) else {
            return;
        };
        let runtime = Arc::downgrade(runtime);
        entries.retain(|entry| entry.strong_count() > 0 && !Weak::ptr_eq(entry, &runtime));
        if entries.is_empty() {
            runtimes.remove(&thread_id);
        }
    }

    pub async fn list(&self, thread_id: ThreadId) -> Vec<TimerInfo> {
        let mut timers = Vec::new();
        for runtime in self.runtimes(thread_id).await {
            timers.extend(runtime.list().await);
        }
        timers.sort_by(|left, right| left.id.cmp(&right.id));
        timers
    }

    pub async fn clear(&self, thread_id: ThreadId) -> u32 {
        let mut cleared = 0_u32;
        for runtime in self.runtimes(thread_id).await {
            cleared = cleared.saturating_add(runtime.clear().await);
        }
        cleared
    }

    pub async fn status(&self, thread_id: ThreadId) -> TimerStatus {
        let mut status = TimerStatus::default();
        for runtime in self.runtimes(thread_id).await {
            let runtime_status = runtime.status().await;
            status.active_count = status
                .active_count
                .saturating_add(runtime_status.active_count);
            status.next_fire_at = match (status.next_fire_at, runtime_status.next_fire_at) {
                (Some(current), Some(candidate)) => Some(current.min(candidate)),
                (Some(current), None) => Some(current),
                (None, candidate) => candidate,
            };
        }
        status
    }

    async fn runtimes(&self, thread_id: ThreadId) -> Vec<Arc<TimerRuntime>> {
        let mut runtimes = self.runtimes.lock().await;
        let Some(entries) = runtimes.get_mut(&thread_id) else {
            return Vec::new();
        };
        let live = entries.iter().filter_map(Weak::upgrade).collect::<Vec<_>>();
        entries.retain(|entry| entry.strong_count() > 0);
        if entries.is_empty() {
            runtimes.remove(&thread_id);
        }
        live
    }
}

struct TimerEntry {
    reminder: String,
    interval: Option<Duration>,
    next_deadline: Instant,
    next_fire_at: i64,
}

#[derive(Clone)]
struct PendingReminder {
    reminder: String,
}

#[derive(Default)]
struct TimerState {
    timers: BTreeMap<String, TimerEntry>,
    pending: BTreeMap<String, PendingReminder>,
    stopped: bool,
}

pub(crate) struct TimerRuntime {
    thread_id: ThreadId,
    thread_manager: Weak<ThreadManager>,
    event_sink: Arc<dyn ExtensionEventSink>,
    state: Mutex<TimerState>,
    dispatch_lock: Semaphore,
    schedule_tx: watch::Sender<u64>,
    schedule_rx: watch::Receiver<u64>,
    scheduler_started: AtomicBool,
}

impl TimerRuntime {
    pub(crate) fn new(
        thread_id: ThreadId,
        thread_manager: Weak<ThreadManager>,
        event_sink: Arc<dyn ExtensionEventSink>,
    ) -> Self {
        let (schedule_tx, schedule_rx) = watch::channel(0);
        Self {
            thread_id,
            thread_manager,
            event_sink,
            state: Mutex::new(TimerState::default()),
            dispatch_lock: Semaphore::new(/*permits*/ 1),
            schedule_tx,
            schedule_rx,
            scheduler_started: AtomicBool::new(false),
        }
    }

    pub(crate) fn start_scheduler(self: &Arc<Self>) {
        if self.scheduler_started.swap(true, Ordering::AcqRel) {
            return;
        }
        tokio::spawn(Self::run_scheduler(
            Arc::downgrade(self),
            self.schedule_rx.clone(),
        ));
    }

    pub(crate) fn thread_id(&self) -> ThreadId {
        self.thread_id
    }

    pub(crate) async fn set(
        self: &Arc<Self>,
        delay_ms: u64,
        reminder: String,
        interval_ms: Option<u64>,
    ) -> Result<TimerInfo, String> {
        validate_timer(delay_ms, &reminder, interval_ms)?;
        let duration = Duration::from_millis(delay_ms);
        let deadline = Instant::now() + duration;
        let next_fire_at = unix_timestamp_after(duration)?;
        let timer_id = Uuid::now_v7().to_string();

        let mut state = self.state.lock().await;
        if state.stopped {
            return Err("timer runtime is stopped".to_string());
        }
        let pending_only = state
            .pending
            .keys()
            .filter(|timer_id| !state.timers.contains_key(*timer_id))
            .count();
        if state.timers.len().saturating_add(pending_only) >= MAX_TIMERS_PER_THREAD {
            return Err(format!(
                "at most {MAX_TIMERS_PER_THREAD} active timers are allowed per thread"
            ));
        }
        state.timers.insert(
            timer_id.clone(),
            TimerEntry {
                reminder: reminder.clone(),
                interval: interval_ms.map(Duration::from_millis),
                next_deadline: deadline,
                next_fire_at,
            },
        );
        let snapshot = status_snapshot(&state);
        drop(state);
        self.notify_scheduler();
        self.emit_snapshot(snapshot);

        Ok(TimerInfo {
            id: timer_id,
            reminder,
            next_fire_at,
            interval_ms,
        })
    }

    pub(crate) async fn cancel(&self, timer_id: &str) -> bool {
        let Ok(_dispatch) = self.dispatch_lock.acquire().await else {
            return false;
        };
        let mut state = self.state.lock().await;
        let timer = state.timers.remove(timer_id);
        let pending = state.pending.remove(timer_id).is_some();
        let cancelled = timer.is_some() || pending;
        let snapshot = status_snapshot(&state);
        drop(state);
        if cancelled {
            self.notify_scheduler();
            self.emit_snapshot(snapshot);
        }
        cancelled
    }

    pub(crate) async fn list(&self) -> Vec<TimerInfo> {
        self.state
            .lock()
            .await
            .timers
            .iter()
            .map(|(id, timer)| TimerInfo {
                id: id.clone(),
                reminder: timer.reminder.clone(),
                next_fire_at: timer.next_fire_at,
                interval_ms: timer.interval.map(duration_millis),
            })
            .collect()
    }

    pub(crate) async fn clear(&self) -> u32 {
        let Ok(_dispatch) = self.dispatch_lock.acquire().await else {
            return 0;
        };
        let mut state = self.state.lock().await;
        let timer_count = state.timers.len();
        let pending_only = state
            .pending
            .keys()
            .filter(|timer_id| !state.timers.contains_key(*timer_id))
            .count();
        state.timers.clear();
        state.pending.clear();
        let cleared = timer_count.saturating_add(pending_only);
        drop(state);
        self.notify_scheduler();
        self.emit_snapshot(TimerStatus::default());
        u32::try_from(cleared).unwrap_or(u32::MAX)
    }

    pub(crate) async fn shutdown(&self) {
        let Ok(_dispatch) = self.dispatch_lock.acquire().await else {
            return;
        };
        let mut state = self.state.lock().await;
        state.stopped = true;
        state.timers.clear();
        state.pending.clear();
        drop(state);
        self.notify_scheduler();
    }

    pub(crate) async fn emit_status(&self) {
        let snapshot = self.status().await;
        self.emit_snapshot(snapshot);
    }

    pub(crate) async fn has_pending_work(&self) -> bool {
        let state = self.state.lock().await;
        !state.timers.is_empty() || !state.pending.is_empty()
    }

    async fn status(&self) -> TimerStatus {
        let state = self.state.lock().await;
        status_snapshot(&state)
    }

    pub(crate) async fn dispatch_pending(&self) {
        let Ok(_dispatch) = self.dispatch_lock.acquire().await else {
            return;
        };
        let pending = {
            let state = self.state.lock().await;
            let mut batch = Vec::new();
            let mut bytes: usize = 0;
            for (timer_id, pending) in &state.pending {
                let entry_bytes = timer_id
                    .len()
                    .saturating_add(pending.reminder.len())
                    .saturating_add(64);
                if !batch.is_empty() && bytes.saturating_add(entry_bytes) > MAX_REMINDER_BATCH_BYTES
                {
                    break;
                }
                bytes = bytes.saturating_add(entry_bytes);
                batch.push((timer_id.clone(), pending.reminder.clone()));
            }
            batch
        };
        if pending.is_empty() {
            return;
        }
        let Some(thread_manager) = self.thread_manager.upgrade() else {
            return;
        };
        let Ok(thread) = thread_manager.get_thread(self.thread_id).await else {
            return;
        };
        match thread.start_timer_turn_if_idle(pending.clone()).await {
            Ok(StartIfIdleSubmission::Started { .. }) => {
                let mut state = self.state.lock().await;
                for (timer_id, _) in pending {
                    state.pending.remove(&timer_id);
                }
            }
            Ok(StartIfIdleSubmission::NotSubmitted { reason }) => {
                tracing::debug!(
                    ?reason,
                    "timer reminders are waiting for the thread to become idle"
                );
            }
            Err(error) => {
                tracing::warn!(%error, "failed to submit timer reminders");
            }
        }
    }

    async fn run_scheduler(weak_runtime: Weak<Self>, mut schedule_rx: watch::Receiver<u64>) {
        loop {
            let Some(runtime) = weak_runtime.upgrade() else {
                return;
            };
            let (stopped, next_deadline) = {
                let state = runtime.state.lock().await;
                (
                    state.stopped,
                    state.timers.values().map(|timer| timer.next_deadline).min(),
                )
            };
            if stopped {
                return;
            }
            drop(runtime);

            match next_deadline {
                Some(deadline) => {
                    tokio::select! {
                        _ = tokio::time::sleep_until(deadline) => {}
                        changed = schedule_rx.changed() => {
                            if changed.is_err() {
                                return;
                            }
                            continue;
                        }
                    }
                }
                None => {
                    if schedule_rx.changed().await.is_err() {
                        return;
                    }
                    continue;
                }
            }

            let Some(runtime) = weak_runtime.upgrade() else {
                return;
            };
            runtime.fire_due().await;
        }
    }

    async fn fire_due(&self) {
        let now = Instant::now();
        let snapshot = {
            let mut state = self.state.lock().await;
            if state.stopped {
                return;
            }
            let due = state
                .timers
                .iter()
                .filter(|(_, timer)| timer.next_deadline <= now)
                .map(|(timer_id, timer)| (timer_id.clone(), timer.reminder.clone(), timer.interval))
                .collect::<Vec<_>>();
            for (timer_id, reminder, interval) in due {
                state
                    .pending
                    .entry(timer_id.clone())
                    .or_insert(PendingReminder { reminder });
                if let Some(interval) = interval {
                    let next_deadline = now + interval;
                    let next_fire_at = unix_timestamp_after(interval).unwrap_or(i64::MAX);
                    let Some(timer) = state.timers.get_mut(&timer_id) else {
                        continue;
                    };
                    timer.next_deadline = next_deadline;
                    timer.next_fire_at = next_fire_at;
                } else {
                    state.timers.remove(&timer_id);
                }
            }
            status_snapshot(&state)
        };
        self.emit_snapshot(snapshot);
        self.dispatch_pending().await;
    }

    fn notify_scheduler(&self) {
        self.schedule_tx.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
    }

    fn emit_snapshot(&self, status: TimerStatus) {
        self.event_sink.emit(Event {
            id: format!("{}:timer-status", self.thread_id),
            msg: EventMsg::TimerStatus(TimerStatusEvent {
                thread_id: self.thread_id,
                active_count: status.active_count,
                next_fire_at: status.next_fire_at,
            }),
        });
    }
}

fn validate_timer(delay_ms: u64, reminder: &str, interval_ms: Option<u64>) -> Result<(), String> {
    if delay_ms > MAX_DELAY_MS {
        return Err(format!("delay_ms must be at most {MAX_DELAY_MS}"));
    }
    if reminder.trim().is_empty() {
        return Err("reminder must not be empty".to_string());
    }
    if reminder.len() > MAX_REMINDER_BYTES {
        return Err(format!(
            "reminder must be at most {MAX_REMINDER_BYTES} UTF-8 bytes"
        ));
    }
    if let Some(interval_ms) = interval_ms {
        if interval_ms < MIN_INTERVAL_MS {
            return Err(format!(
                "interval_ms must be at least {MIN_INTERVAL_MS} for periodic timers"
            ));
        }
        if interval_ms > MAX_DELAY_MS {
            return Err(format!("interval_ms must be at most {MAX_DELAY_MS}"));
        }
    }
    Ok(())
}

fn unix_timestamp_after(duration: Duration) -> Result<i64, String> {
    let fire_at = SystemTime::now()
        .checked_add(duration)
        .ok_or_else(|| "timer deadline is out of range".to_string())?
        .duration_since(UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_secs();
    i64::try_from(fire_at).map_err(|_| "timer deadline is out of range".to_string())
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn status_snapshot(state: &TimerState) -> TimerStatus {
    let active_count = u32::try_from(state.timers.len()).unwrap_or(u32::MAX);
    let next_fire_at = state.timers.values().map(|timer| timer.next_fire_at).min();
    TimerStatus {
        active_count,
        next_fire_at,
    }
}
