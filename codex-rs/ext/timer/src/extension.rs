use std::sync::Arc;
use std::sync::Weak;

use codex_core::ThreadManager;
use codex_extension_api::ExtensionData;
use codex_extension_api::ExtensionEventSink;
use codex_extension_api::ExtensionFuture;
use codex_extension_api::ThreadIdleInput;
use codex_extension_api::ThreadLifecycleContributor;
use codex_extension_api::ThreadResumeInput;
use codex_extension_api::ThreadStartInput;
use codex_extension_api::ThreadStopInput;
use codex_extension_api::ToolCall;
use codex_extension_api::ToolContributor;
use codex_extension_api::ToolExecutor;
use codex_protocol::ThreadId;

use crate::runtime::TimerRuntime;
use crate::runtime::TimerService;
use crate::tool::TimerTool;

#[derive(Clone)]
pub struct TimerExtension {
    service: Arc<TimerService>,
    thread_manager: Weak<ThreadManager>,
    event_sink: Arc<dyn ExtensionEventSink>,
}

impl TimerExtension {
    pub(crate) fn new(
        service: Arc<TimerService>,
        thread_manager: Weak<ThreadManager>,
        event_sink: Arc<dyn ExtensionEventSink>,
    ) -> Self {
        Self {
            service,
            thread_manager,
            event_sink,
        }
    }
}

impl<C> ThreadLifecycleContributor<C> for TimerExtension
where
    C: Send + Sync + 'static,
{
    fn on_thread_start<'a>(&'a self, input: ThreadStartInput<'a, C>) -> ExtensionFuture<'a, ()> {
        Box::pin(async move {
            let Ok(thread_id) = ThreadId::from_string(input.thread_store.level_id()) else {
                return;
            };
            let runtime = input.thread_store.get_or_init::<TimerRuntime>(|| {
                TimerRuntime::new(
                    thread_id,
                    self.thread_manager.clone(),
                    Arc::clone(&self.event_sink),
                )
            });
            runtime.start_scheduler();
            self.service.register(thread_id, &runtime).await;
            runtime.emit_status().await;
        })
    }

    fn on_thread_resume<'a>(&'a self, input: ThreadResumeInput<'a>) -> ExtensionFuture<'a, ()> {
        Box::pin(async move {
            if let Some(runtime) = input.thread_store.get::<TimerRuntime>() {
                runtime.emit_status().await;
            }
        })
    }

    fn on_thread_idle<'a>(&'a self, input: ThreadIdleInput<'a>) -> ExtensionFuture<'a, ()> {
        Box::pin(async move {
            if let Some(runtime) = input.thread_store.get::<TimerRuntime>() {
                runtime.emit_status().await;
                runtime.dispatch_pending().await;
            }
        })
    }

    fn on_thread_stop<'a>(&'a self, input: ThreadStopInput<'a>) -> ExtensionFuture<'a, ()> {
        Box::pin(async move {
            let Some(runtime) = input.thread_store.get::<TimerRuntime>() else {
                return;
            };
            runtime.shutdown().await;
            self.service.unregister(runtime.thread_id(), &runtime).await;
        })
    }
}

impl ToolContributor for TimerExtension {
    fn tools(
        &self,
        _session_store: &ExtensionData,
        thread_store: &ExtensionData,
    ) -> Vec<Arc<dyn ToolExecutor<ToolCall>>> {
        let Some(runtime) = thread_store.get::<TimerRuntime>() else {
            return Vec::new();
        };
        TimerTool::all(runtime)
            .into_iter()
            .map(|tool| Arc::new(tool) as Arc<dyn ToolExecutor<ToolCall>>)
            .collect()
    }
}
