use super::should_persist_event_msg;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ThreadHistoryMode;
use pretty_assertions::assert_eq;

#[test]
fn completed_response_usage_is_durable_and_old_events_remain_readable() {
    let event: EventMsg = serde_json::from_value(serde_json::json!({
        "type": "raw_response_completed",
        "response_id": "old-response",
        "token_usage": null,
    }))
    .expect("events without request metadata remain readable");
    let EventMsg::RawResponseCompleted(completed) = &event else {
        panic!("expected response completion");
    };
    assert_eq!(
        (&completed.model, &completed.reasoning_effort),
        (&None, &None)
    );
    for mode in [ThreadHistoryMode::Legacy, ThreadHistoryMode::Paginated] {
        assert!(should_persist_event_msg(&event, mode));
    }
}

#[test]
fn persisted_scrollback_bounds_large_command_output_without_altering_live_event() {
    let text = "€".repeat(30_000);
    let event: EventMsg = serde_json::from_value(serde_json::json!({
        "type": "item_completed", "thread_id": codex_protocol::ThreadId::new(), "turn_id": "turn",
        "item": { "type": "CommandExecution", "id": "cmd", "command": ["test"], "cwd": "file:///tmp",
            "parsed_cmd": [], "source": "agent", "status": "completed", "aggregated_output": text },
    })).unwrap();
    let item = super::RolloutItem::EventMsg(event);
    let stored =
        super::persisted_rollout_items(std::slice::from_ref(&item), ThreadHistoryMode::Paginated);
    let stored = serde_json::to_value(&stored[0]).unwrap();
    let live = serde_json::to_value(&item).unwrap();
    assert_eq!(live["payload"]["item"]["aggregated_output"], text);
    assert_eq!(
        stored["payload"]["item"]["aggregated_output"],
        format!("{}\n[Stored output truncated]", "€".repeat(5461))
    );
}
