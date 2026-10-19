//! Responses API tool definitions for persisted thread goals.

use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use serde_json::json;
use std::collections::BTreeMap;

pub const GET_GOAL_TOOL_NAME: &str = "get_goal";
pub const UPDATE_GOAL_TOOL_NAME: &str = "update_goal";

pub fn create_get_goal_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: GET_GOAL_TOOL_NAME.to_string(),
        description: "Get the current goal for this thread, including status, budgets, token and elapsed-time usage, and remaining token budget."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(BTreeMap::new(), Some(Vec::new()), Some(false.into())),
        output_schema: None,
    })
}

pub fn create_update_goal_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "objective".to_string(),
            JsonSchema::string(Some(
                "A revised objective reflecting the user's changed direction. Omit when only changing status."
                    .to_string(),
            )),
        ),
        (
            "status".to_string(),
            JsonSchema::string_enum(
                vec![json!("complete"), json!("blocked"), json!("cancelled")],
                Some(
                    "Set to `complete` only when achieved, `blocked` only after the required repeated-blocker audit, or `cancelled` when the user has withdrawn the goal."
                        .to_string(),
                ),
            ),
        ),
    ]);
    let mut parameters = JsonSchema::object(
        properties,
        /*required*/ Some(Vec::new()),
        Some(false.into()),
    );
    parameters.any_of = Some(vec![
        JsonSchema {
            required: Some(vec!["objective".to_string()]),
            ..Default::default()
        },
        JsonSchema {
            required: Some(vec!["status".to_string()]),
            ..Default::default()
        },
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: UPDATE_GOAL_TOOL_NAME.to_string(),
        description: r#"Update the existing goal's objective or status. Provide at least one field.
Revise `objective` when the user's direction changes while the goal remains active.
Set status to `cancelled` when the user withdraws the goal; this stops automatic continuation without reporting completion.
Set status to `complete` only when the objective has actually been achieved and no required work remains.
Set status to `blocked` only when the same blocking condition has repeated for at least three consecutive goal turns, counting the original/user-triggered turn and any automatic continuations, and the agent cannot make meaningful progress without user input or an external-state change.
If the user resumes a goal that was previously marked `blocked`, treat the resumed run as a fresh blocked audit. If the same blocking condition then repeats for at least three consecutive resumed goal turns, set status to `blocked` again.
Once the blocked threshold is satisfied, do not keep reporting that you are still blocked while leaving the goal active; set status to `blocked`.
Do not use `blocked` merely because the work is hard, slow, uncertain, incomplete, or would benefit from clarification.
Do not mark a goal complete merely because its budget is nearly exhausted or because you are stopping work.
You cannot use this tool to pause, resume, budget-limit, or usage-limit a goal; those status changes are controlled by the user or system.
When marking a budgeted goal achieved with status `complete`, report the final token usage from the tool result to the user."#
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters,
        output_schema: None,
    })
}
