use super::*;
use pretty_assertions::assert_eq;

#[test]
fn daily_rollover_keeps_counting_work_from_already_running_turns() {
    let mut counters = DailyTokenUsage::default();
    let usage = TokenUsage {
        input_tokens: 100,
        cached_input_tokens: 60,
        output_tokens: 20,
        reasoning_output_tokens: 5,
        total_tokens: 120,
    };
    let doubled = TokenUsage {
        input_tokens: 200,
        cached_input_tokens: 120,
        output_tokens: 40,
        reasoning_output_tokens: 10,
        total_tokens: 240,
    };
    for thread in ["main", "worker"] {
        counters.start_turn(thread, "running-turn");
        counters.observe(thread, "running-turn", usage.clone(), usage.clone());
    }
    assert_eq!(counters.total(), doubled.clone());

    counters.bucket = current_daily_bucket().pred_opt().expect("previous day");
    assert!(counters.rollover_on_submission());
    assert_eq!(counters.total(), TokenUsage::default());

    // A steer or a message to another thread does not start a new turn for either agent.
    // Count only the new usage since the rollover, without replaying yesterday's totals.
    for thread in ["main", "worker"] {
        counters.observe(thread, "running-turn", doubled.clone(), usage.clone());
    }
    assert_eq!(counters.total(), doubled);
    assert!(!counters.rollover_on_submission());
}
