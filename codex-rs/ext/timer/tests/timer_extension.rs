#![allow(clippy::expect_used)]

use std::sync::Arc;
use std::sync::Weak;
use std::time::Duration;

use codex_extension_api::ExtensionData;
use codex_extension_api::ExtensionRegistry;
use codex_extension_api::ExtensionRegistryBuilder;
use codex_extension_api::NoopExtensionEventSink;
use codex_extension_api::NoopTurnItemEmitter;
use codex_extension_api::ThreadStartInput;
use codex_extension_api::ThreadStopInput;
use codex_extension_api::ToolCall;
use codex_extension_api::ToolPayload;
use codex_protocol::ThreadId;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::TruncationPolicy;
use codex_timer_extension::TimerInfo;
use codex_timer_extension::TimerService;
use codex_timer_extension::install;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;

struct TimerHarness {
    thread_id: ThreadId,
    service: Arc<TimerService>,
    registry: ExtensionRegistry<()>,
    session_store: ExtensionData,
    thread_store: ExtensionData,
}

impl TimerHarness {
    async fn new() -> Self {
        let thread_id = ThreadId::new();
        let service = Arc::new(TimerService::new());
        let mut builder =
            ExtensionRegistryBuilder::<()>::with_event_sink(Arc::new(NoopExtensionEventSink));
        install(
            &mut builder,
            Arc::clone(&service),
            /*thread_manager*/ Weak::new(),
        );
        let registry = builder.build();
        let session_store = ExtensionData::new("session");
        let thread_store = ExtensionData::new(thread_id.to_string());
        let session_source = SessionSource::Cli;
        for contributor in registry.thread_lifecycle_contributors() {
            contributor
                .on_thread_start(ThreadStartInput {
                    config: &(),
                    session_source: &session_source,
                    persistent_thread_state_available: false,
                    environments: &[],
                    mcp_resource_client: None,
                    extension_metrics: None,
                    session_store: &session_store,
                    thread_store: &thread_store,
                })
                .await;
        }
        Self {
            thread_id,
            service,
            registry,
            session_store,
            thread_store,
        }
    }

    async fn call(&self, name: &str, arguments: Value) {
        let tool = self
            .registry
            .tool_contributors()
            .iter()
            .flat_map(|contributor| contributor.tools(&self.session_store, &self.thread_store))
            .find(|tool| {
                let tool_name = tool.tool_name();
                tool_name.namespace.as_deref() == Some("timer") && tool_name.name == name
            })
            .unwrap_or_else(|| panic!("timer.{name} should be installed"));
        tool.handle(tool_call(name, arguments))
            .await
            .unwrap_or_else(|error| panic!("timer.{name} should succeed: {error}"));
    }

    async fn stop_thread(&self) {
        for contributor in self.registry.thread_lifecycle_contributors() {
            contributor
                .on_thread_stop(ThreadStopInput {
                    session_store: &self.session_store,
                    thread_store: &self.thread_store,
                })
                .await;
        }
    }
}

#[tokio::test(start_paused = true)]
async fn scheduler_collects_due_reminders_and_cancel_prevents_delivery() {
    let harness = TimerHarness::new().await;

    harness
        .call("set", json!({"delay_ms": 10, "reminder": "first"}))
        .await;
    harness
        .call("set", json!({"delay_ms": 10, "reminder": "second"}))
        .await;
    assert_eq!(2, harness.service.list(harness.thread_id).await.len());

    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(10)).await;
    for _ in 0..10 {
        if harness.service.list(harness.thread_id).await.is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }

    assert_eq!(
        Vec::<TimerInfo>::new(),
        harness.service.list(harness.thread_id).await
    );
    assert_eq!(2, harness.service.clear(harness.thread_id).await);

    harness
        .call("set", json!({"delay_ms": 10, "reminder": "cancel me"}))
        .await;
    let timer_id = harness.service.list(harness.thread_id).await[0].id.clone();
    harness.call("cancel", json!({"timer_id": timer_id})).await;

    tokio::time::advance(Duration::from_millis(10)).await;
    tokio::task::yield_now().await;
    assert_eq!(0, harness.service.clear(harness.thread_id).await);

    harness
        .call("set", json!({"delay_ms": 60_000, "reminder": "never fire"}))
        .await;
    harness.stop_thread().await;
    assert_eq!(
        Vec::<TimerInfo>::new(),
        harness.service.list(harness.thread_id).await
    );
    assert_eq!(0, harness.service.clear(harness.thread_id).await);
}

fn tool_call(tool_name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        turn_id: "turn-1".to_string(),
        call_id: format!("call-{tool_name}"),
        tool_name: codex_extension_api::ToolName::namespaced("timer", tool_name),
        model: "gpt-test".to_string(),
        codex_turn_metadata: None,
        truncation_policy: TruncationPolicy::Bytes(1_024),
        conversation_history: codex_extension_api::ConversationHistory::default(),
        turn_item_emitter: Arc::new(NoopTurnItemEmitter),
        environments: Vec::new(),
        payload: ToolPayload::Function {
            arguments: arguments.to_string(),
        },
    }
}
