use crate::agent::AgentControl;
use codex_protocol::error::CodexErrorDetails;
use codex_protocol::protocol::MultiAgentVersion;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use pretty_assertions::assert_eq;

fn control_with_limit(max_threads: usize) -> AgentControl {
    let control = AgentControl::default();
    control.agent_execution_limiter.initialize(max_threads);
    control
}

#[test]
fn execution_guards_count_active_v2_subagent_turns() {
    let control = control_with_limit(/*max_threads*/ 1);
    // Child role configs cannot replace the root-derived session limit.
    control
        .agent_execution_limiter
        .initialize(/*max_threads*/ 2);
    let source = SessionSource::SubAgent(SubAgentSource::Other("worker".to_string()));

    control
        .ensure_execution_capacity(MultiAgentVersion::V2, &source)
        .expect("first active turn should fit");
    let first = control
        .execution_guard(MultiAgentVersion::V2, &source)
        .expect("v2 subagent execution should be counted");
    let Err(err) = control.ensure_execution_capacity(MultiAgentVersion::V2, &source) else {
        panic!("second active turn should exceed the derived non-root cap");
    };
    let CodexErrorDetails::AgentLimitReached { max_threads } = err.details() else {
        panic!("expected AgentLimitReached");
    };
    assert_eq!(*max_threads, 1);

    drop(first);
    control
        .ensure_execution_capacity(MultiAgentVersion::V2, &source)
        .expect("capacity should be released when the running task drops");
}

#[test]
fn execution_guards_ignore_root_and_v1_turns() {
    let control = control_with_limit(/*max_threads*/ 0);

    assert!(
        control
            .execution_guard(MultiAgentVersion::V2, &SessionSource::Cli)
            .is_none()
    );
    assert!(
        control
            .execution_guard(
                MultiAgentVersion::V1,
                &SessionSource::SubAgent(SubAgentSource::Other("worker".to_string())),
            )
            .is_none()
    );
}

#[test]
fn live_agent_limit_can_increase_and_decrease_without_stopping_active_work() {
    let control = control_with_limit(/*max_threads*/ 1);
    let source = SessionSource::SubAgent(SubAgentSource::Other("worker".to_string()));
    let active = control
        .execution_guard(MultiAgentVersion::V2, &source)
        .expect("count active worker");
    assert!(
        control
            .ensure_execution_capacity(MultiAgentVersion::V2, &source)
            .is_err()
    );
    control.set_agent_limit(3);
    control
        .ensure_execution_capacity(MultiAgentVersion::V2, &source)
        .expect("new capacity");
    control.set_agent_limit(1);
    assert!(
        control
            .ensure_execution_capacity(MultiAgentVersion::V2, &source)
            .is_err()
    );
    drop(active);
    assert!(
        control
            .ensure_execution_capacity(MultiAgentVersion::V2, &source)
            .is_err()
    );
    control.set_agent_limit(2);
    control
        .ensure_execution_capacity(MultiAgentVersion::V2, &source)
        .expect("capacity restored");
}
