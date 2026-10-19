use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn limit_agents_command_displays_session_cap_and_accepts_valid_limit() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(Some("gpt-6.1-sol")).await;
    chat.set_agent_limit(4);
    while rx.try_recv().is_ok() {}
    chat.dispatch_command(SlashCommand::LimitAgents);
    let cell = match rx.try_recv().expect("limit info") {
        AppEvent::InsertHistoryCell(cell) => cell,
        other => panic!("unexpected event {other:?}"),
    };
    insta::assert_snapshot!(
        "limit_agents_help",
        cell.display_lines(/*width*/ 100)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    );
    chat.dispatch_command_with_args(SlashCommand::LimitAgents, "8".to_string(), Vec::new());
    assert!(matches!(rx.try_recv(), Ok(AppEvent::SetAgentLimit(8))));
    for invalid in ["0", "-1", "three", "2 3"] {
        chat.dispatch_command_with_args(SlashCommand::LimitAgents, invalid.to_string(), Vec::new());
        assert!(matches!(rx.try_recv(), Ok(AppEvent::InsertHistoryCell(_))));
    }
    assert_eq!(
        chat.config
            .multi_agent_v2
            .max_concurrent_threads_per_session,
        4
    );
}
