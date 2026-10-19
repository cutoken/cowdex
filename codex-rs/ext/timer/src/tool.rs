use std::collections::BTreeMap;
use std::sync::Arc;

use codex_extension_api::FunctionCallError;
use codex_extension_api::JsonToolOutput;
use codex_extension_api::ToolCall;
use codex_extension_api::ToolExecutor;
use codex_extension_api::ToolName;
use codex_extension_api::ToolOutput;
use codex_extension_api::ToolSpec;
use codex_tools::JsonSchema;
use codex_tools::ResponsesApiNamespace;
use codex_tools::ResponsesApiNamespaceTool;
use codex_tools::ResponsesApiTool;
use serde::Deserialize;
use serde_json::json;

use crate::CANCEL_TOOL_NAME;
use crate::LIST_TOOL_NAME;
use crate::SET_TOOL_NAME;
use crate::TIMER_NAMESPACE;
use crate::runtime::TimerRuntime;

#[derive(Clone, Copy)]
enum TimerToolKind {
    Set,
    Cancel,
    List,
}

pub(crate) struct TimerTool {
    runtime: Arc<TimerRuntime>,
    kind: TimerToolKind,
}

#[derive(Deserialize)]
struct SetArgs {
    delay_ms: u64,
    reminder: String,
    interval_ms: Option<u64>,
}

#[derive(Deserialize)]
struct CancelArgs {
    timer_id: String,
}

impl TimerTool {
    pub(crate) fn all(runtime: Arc<TimerRuntime>) -> [Self; 3] {
        [
            TimerToolKind::Set,
            TimerToolKind::Cancel,
            TimerToolKind::List,
        ]
        .map(|kind| Self {
            runtime: Arc::clone(&runtime),
            kind,
        })
    }
}

impl ToolExecutor<ToolCall> for TimerTool {
    fn tool_name(&self) -> ToolName {
        ToolName::namespaced(TIMER_NAMESPACE, self.name())
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec::Namespace(ResponsesApiNamespace {
            name: TIMER_NAMESPACE.to_string(),
            description: "Schedule reminders that wake this thread after the current turn has ended. Timers live only for the current process session."
                .to_string(),
            tools: vec![ResponsesApiNamespaceTool::Function(ResponsesApiTool {
                name: self.name().to_string(),
                description: self.description().to_string(),
                strict: false,
                parameters: self.parameters(),
                output_schema: None,
                defer_loading: None,
            })],
        })
    }

    fn handle(&self, invocation: ToolCall) -> codex_extension_api::ToolExecutorFuture<'_> {
        Box::pin(async move {
            match self.kind {
                TimerToolKind::Set => {
                    let args: SetArgs = parse_args(invocation.function_arguments()?)?;
                    let timer = self
                        .runtime
                        .set(args.delay_ms, args.reminder, args.interval_ms)
                        .await
                        .map_err(FunctionCallError::RespondToModel)?;
                    json_output(json!({ "timer": timer }))
                }
                TimerToolKind::Cancel => {
                    let args: CancelArgs = parse_args(invocation.function_arguments()?)?;
                    let cancelled = self.runtime.cancel(&args.timer_id).await;
                    json_output(json!({ "cancelled": cancelled }))
                }
                TimerToolKind::List => {
                    let _: serde_json::Value = parse_args(invocation.function_arguments()?)?;
                    let timers = self.runtime.list().await;
                    json_output(json!({ "timers": timers }))
                }
            }
        })
    }
}

impl TimerTool {
    fn name(&self) -> &'static str {
        match self.kind {
            TimerToolKind::Set => SET_TOOL_NAME,
            TimerToolKind::Cancel => CANCEL_TOOL_NAME,
            TimerToolKind::List => LIST_TOOL_NAME,
        }
    }

    fn description(&self) -> &'static str {
        match self.kind {
            TimerToolKind::Set => {
                "Schedule a one-shot or periodic reminder for this thread. Periodic intervals must be at least 60000 ms."
            }
            TimerToolKind::Cancel => {
                "Cancel one scheduled or fired-but-not-yet-delivered timer by id."
            }
            TimerToolKind::List => "List active timers for this thread.",
        }
    }

    fn parameters(&self) -> JsonSchema {
        match self.kind {
            TimerToolKind::Set => JsonSchema::object(
                BTreeMap::from([
                    (
                        "delay_ms".to_string(),
                        JsonSchema::integer(Some(
                            "Delay before the first reminder, in milliseconds.".to_string(),
                        )),
                    ),
                    (
                        "reminder".to_string(),
                        JsonSchema::string(Some(
                            "Reminder text delivered when the timer fires.".to_string(),
                        )),
                    ),
                    (
                        "interval_ms".to_string(),
                        JsonSchema::integer(Some(
                            "Optional repeat interval in milliseconds; omit for one-shot."
                                .to_string(),
                        )),
                    ),
                ]),
                Some(vec!["delay_ms".to_string(), "reminder".to_string()]),
                Some(false.into()),
            ),
            TimerToolKind::Cancel => JsonSchema::object(
                BTreeMap::from([(
                    "timer_id".to_string(),
                    JsonSchema::string(Some("Timer identifier returned by timer.set.".to_string())),
                )]),
                Some(vec!["timer_id".to_string()]),
                Some(false.into()),
            ),
            TimerToolKind::List => {
                JsonSchema::object(BTreeMap::new(), Some(Vec::new()), Some(false.into()))
            }
        }
    }
}

fn parse_args<T>(arguments: &str) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(arguments)
        .map_err(|err| FunctionCallError::RespondToModel(format!("invalid timer arguments: {err}")))
}

fn json_output(value: serde_json::Value) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    Ok(Box::new(JsonToolOutput::new(value)))
}
