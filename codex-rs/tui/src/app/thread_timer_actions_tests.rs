use super::*;

#[test]
fn timer_list_copy_is_compact_and_explains_recurrence() {
    let timers = vec![
        ThreadTimer {
            id: "timer-1".to_string(),
            reminder: "Send the report".to_string(),
            next_fire_at: 1_800_000_000,
            interval_ms: None,
        },
        ThreadTimer {
            id: "timer-2".to_string(),
            reminder: "Stand up".to_string(),
            next_fire_at: 1_800_003_600,
            interval_ms: Some(3_600_000),
        },
    ];
    let rendered = timer_list_lines(&timers)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    insta::assert_snapshot!(rendered, @r"
    Timers
    • Send the report · 15 Jan 2027, 08:00 UTC
    • Stand up · 15 Jan 2027, 09:00 UTC · every 1h
    ");
}
