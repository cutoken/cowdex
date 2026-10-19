//! Session-lifetime timers that wake an idle Codex thread.

mod extension;
mod runtime;
mod tool;

use std::sync::Arc;
use std::sync::Weak;

use codex_core::ThreadManager;
use codex_extension_api::ExtensionRegistryBuilder;

pub use extension::TimerExtension;
pub use runtime::TimerInfo;
pub use runtime::TimerService;
pub use runtime::TimerStatus;

pub const TIMER_NAMESPACE: &str = "timer";
pub const SET_TOOL_NAME: &str = "set";
pub const CANCEL_TOOL_NAME: &str = "cancel";
pub const LIST_TOOL_NAME: &str = "list";

/// Returns whether a live thread runtime still has scheduled or undelivered timer work.
pub async fn has_pending_work(thread_store: &codex_extension_api::ExtensionData) -> bool {
    let Some(runtime) = thread_store.get::<runtime::TimerRuntime>() else {
        return false;
    };
    runtime.has_pending_work().await
}

pub fn install<C>(
    registry: &mut ExtensionRegistryBuilder<C>,
    service: Arc<TimerService>,
    thread_manager: Weak<ThreadManager>,
) where
    C: Send + Sync + 'static,
{
    let extension = Arc::new(TimerExtension::new(
        service,
        thread_manager,
        registry.event_sink(),
    ));
    registry.thread_lifecycle_contributor(extension.clone());
    registry.tool_contributor(extension);
}
