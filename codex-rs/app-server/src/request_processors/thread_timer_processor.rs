use std::sync::Arc;

use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::ThreadTimer;
use codex_app_server_protocol::ThreadTimerClearParams;
use codex_app_server_protocol::ThreadTimerClearResponse;
use codex_app_server_protocol::ThreadTimerListParams;
use codex_app_server_protocol::ThreadTimerListResponse;
use codex_protocol::ThreadId;
use codex_timer_extension::TimerInfo;
use codex_timer_extension::TimerService;
use uuid::Uuid;

use crate::error_code::invalid_request;

use super::thread_processor::THREAD_LIST_DEFAULT_LIMIT;
use super::thread_processor::THREAD_LIST_MAX_LIMIT;

#[derive(Clone)]
pub(crate) struct ThreadTimerRequestProcessor {
    service: Arc<TimerService>,
}

impl ThreadTimerRequestProcessor {
    pub(crate) fn new(service: Arc<TimerService>) -> Self {
        Self { service }
    }

    pub(crate) async fn list(
        &self,
        params: ThreadTimerListParams,
    ) -> Result<ThreadTimerListResponse, JSONRPCErrorError> {
        let thread_id = parse_thread_id(&params.thread_id)?;
        let cursor = params
            .cursor
            .as_deref()
            .map(|cursor| Uuid::parse_str(cursor).map(|_| cursor))
            .transpose()
            .map_err(|error| invalid_request(format!("invalid timer pagination cursor: {error}")))?
            .map(str::to_owned);
        let limit = params
            .limit
            .map(|value| value as usize)
            .unwrap_or(THREAD_LIST_DEFAULT_LIMIT)
            .clamp(1, THREAD_LIST_MAX_LIMIT);
        let timers = self.service.list(thread_id).await;
        let start = cursor
            .as_deref()
            .map(|cursor| timers.partition_point(|timer| timer.id.as_str() <= cursor))
            .unwrap_or_default();
        let mut data = timers
            .into_iter()
            .skip(start)
            .take(limit.saturating_add(1))
            .collect::<Vec<_>>();
        let next_cursor = if data.len() > limit {
            data.truncate(limit);
            data.last().map(|timer| timer.id.clone())
        } else {
            None
        };
        Ok(ThreadTimerListResponse {
            data: data.into_iter().map(api_timer).collect(),
            next_cursor,
        })
    }

    pub(crate) async fn clear(
        &self,
        params: ThreadTimerClearParams,
    ) -> Result<ThreadTimerClearResponse, JSONRPCErrorError> {
        let thread_id = parse_thread_id(&params.thread_id)?;
        let cleared = self.service.clear(thread_id).await;
        Ok(ThreadTimerClearResponse { cleared })
    }
}

fn parse_thread_id(thread_id: &str) -> Result<ThreadId, JSONRPCErrorError> {
    ThreadId::from_string(thread_id)
        .map_err(|error| invalid_request(format!("invalid thread id: {error}")))
}

fn api_timer(timer: TimerInfo) -> ThreadTimer {
    ThreadTimer {
        id: timer.id,
        reminder: timer.reminder,
        next_fire_at: timer.next_fire_at,
        interval_ms: timer.interval_ms,
    }
}
