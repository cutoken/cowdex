use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::context::ContextualUserFragment;
use crate::context::SuspendedAgentWaitTimeout;
use crate::session::InputQueueActivity;
use crate::session::TurnInput;
use crate::session::session::Session;
use crate::session::turn::run_hooks_and_record_inputs;
use crate::session::turn::run_turn;
use crate::session::turn_context::TurnContext;
use crate::session_startup_prewarm::SessionStartupPrewarmResolution;
use crate::state::TaskKind;
use codex_protocol::error::CodexErr;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::TurnStartedEvent;
use codex_thread_store::PersistContext;
use tracing::Instrument;
use tracing::trace_span;

use super::SessionTask;
use super::SessionTaskResult;

#[derive(Default)]
pub(crate) struct RegularTask;

impl RegularTask {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl SessionTask for RegularTask {
    fn kind(&self) -> TaskKind {
        TaskKind::Regular
    }

    fn span_name(&self) -> &'static str {
        "session_task.turn"
    }

    async fn run(
        self: Arc<Self>,
        sess: Arc<Session>,
        ctx: Arc<TurnContext>,
        input: Vec<TurnInput>,
        cancellation_token: CancellationToken,
    ) -> SessionTaskResult {
        let run_turn_span = trace_span!("run_turn");
        // Regular turns emit `TurnStarted` inline so first-turn lifecycle does
        // not wait on startup prewarm resolution.
        let prewarmed_client_session = async {
            let event = EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: ctx.sub_id.clone(),
                trace_id: ctx.trace_id.clone(),
                started_at: ctx.turn_timing_state.started_at_unix_secs().await,
                model_context_window: ctx.model_context_window(),
                collaboration_mode_kind: ctx.mode,
            });
            sess.send_event(ctx.as_ref(), event).await;
            sess.set_server_reasoning_included(/*included*/ false).await;
            sess.consume_startup_prewarm_for_regular_turn(&cancellation_token)
                .await
        }
        .instrument(trace_span!("regular_task.prepare_run_turn"))
        .await;
        let prewarmed_client_session = match prewarmed_client_session {
            SessionStartupPrewarmResolution::Cancelled => {
                run_hooks_and_record_inputs(&sess, &ctx, &input, PersistContext::Standard).await;
                return Ok(None);
            }
            SessionStartupPrewarmResolution::Unavailable { .. } => None,
            SessionStartupPrewarmResolution::Ready(prewarmed_client_session) => {
                Some(*prewarmed_client_session)
            }
        };
        let mut next_input = input;
        let mut prewarmed_client_session = prewarmed_client_session;
        loop {
            let last_agent_message = run_turn(
                Arc::clone(&sess),
                Arc::clone(&ctx),
                next_input,
                prewarmed_client_session.take(),
                cancellation_token.child_token(),
            )
            .instrument(run_turn_span.clone())
            .await?;
            if ctx.terminal_error.lock().await.is_some() {
                return Ok(last_agent_message);
            }
            if sess.input_queue.has_pending_input(&sess.active_turn).await {
                next_input = Vec::new();
                continue;
            }
            match wait_for_suspended_agent_wait(&sess, &ctx, &cancellation_token).await? {
                SuspendedWaitOutcome::NoWait => return Ok(last_agent_message),
                SuspendedWaitOutcome::PendingInput => next_input = Vec::new(),
                SuspendedWaitOutcome::TimedOut => {
                    let timeout: ResponseItem =
                        ContextualUserFragment::into(SuspendedAgentWaitTimeout);
                    next_input = vec![TurnInput::ResponseItem(timeout.into())];
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SuspendedWaitOutcome {
    NoWait,
    PendingInput,
    TimedOut,
}

async fn wait_for_suspended_agent_wait(
    sess: &Session,
    ctx: &TurnContext,
    cancellation_token: &CancellationToken,
) -> Result<SuspendedWaitOutcome, CodexErr> {
    let Some(turn_state) = sess
        .input_queue
        .turn_state_for_sub_id(&sess.active_turn, &ctx.sub_id)
        .await
    else {
        return Ok(SuspendedWaitOutcome::NoWait);
    };
    let Some(wait) = turn_state.lock().await.suspended_agent_wait() else {
        return Ok(SuspendedWaitOutcome::NoWait);
    };
    let (mut activity_rx, mut pending_activity) = sess
        .input_queue
        .subscribe_activity(Some(turn_state.as_ref()))
        .await;

    loop {
        if pending_activity == Some(InputQueueActivity::Steer)
            || sess
                .input_queue
                .has_pending_mailbox_from(&wait.awaited_agent_paths)
                .await
        {
            sess.input_queue
                .accept_mailbox_delivery_for_current_turn(&sess.active_turn, &ctx.sub_id)
                .await;
            return Ok(SuspendedWaitOutcome::PendingInput);
        }
        if wait.deadline <= tokio::time::Instant::now() {
            turn_state.lock().await.clear_suspended_agent_wait();
            return Ok(SuspendedWaitOutcome::TimedOut);
        }

        tokio::select! {
            biased;
            _ = cancellation_token.cancelled() => return Err(CodexErr::TurnAborted),
            activity = activity_rx.changed() => {
                if activity.is_err() {
                    tokio::select! {
                        _ = cancellation_token.cancelled() => return Err(CodexErr::TurnAborted),
                        _ = tokio::time::sleep_until(wait.deadline) => {}
                    }
                    turn_state.lock().await.clear_suspended_agent_wait();
                    return Ok(SuspendedWaitOutcome::TimedOut);
                }
                pending_activity = Some(*activity_rx.borrow_and_update());
            }
            _ = tokio::time::sleep_until(wait.deadline) => {
                turn_state.lock().await.clear_suspended_agent_wait();
                return Ok(SuspendedWaitOutcome::TimedOut);
            }
        }
    }
}
