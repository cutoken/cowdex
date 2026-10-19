//! Helpers for mapping thread-goal state into the compact status-line indicator.

use codex_app_server_protocol::ThreadGoal as AppThreadGoal;
use codex_app_server_protocol::ThreadGoalStatus as AppThreadGoalStatus;
use std::time::Instant;

use crate::bottom_pane::GoalStatusIndicator;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct GoalStatusState {
    goal: AppThreadGoal,
    observed_at: Instant,
}

impl GoalStatusState {
    pub(super) fn new(goal: AppThreadGoal, observed_at: Instant) -> Self {
        Self { goal, observed_at }
    }

    pub(super) fn is_active(&self) -> bool {
        self.goal.status == AppThreadGoalStatus::Active
    }

    pub(super) fn indicator(
        &self,
        now: Instant,
        active_turn_started_at: Option<Instant>,
    ) -> Option<GoalStatusIndicator> {
        let mut goal = self.goal.clone();
        if goal.status == AppThreadGoalStatus::Active
            && let Some(active_turn_started_at) = active_turn_started_at
        {
            let baseline = self.observed_at.max(active_turn_started_at);
            let active_seconds = now.saturating_duration_since(baseline).as_secs();
            goal.time_used_seconds = goal
                .time_used_seconds
                .saturating_add(i64::try_from(active_seconds).unwrap_or(i64::MAX));
        }
        goal_status_indicator_from_app_goal(&goal)
    }
}

pub(super) fn goal_status_indicator_from_app_goal(
    goal: &AppThreadGoal,
) -> Option<GoalStatusIndicator> {
    let detail = match goal.status {
        AppThreadGoalStatus::Blocked => "stalled".to_string(),
        AppThreadGoalStatus::UsageLimited => "usage limit".to_string(),
        AppThreadGoalStatus::BudgetLimited => "token budget".to_string(),
        AppThreadGoalStatus::Active
        | AppThreadGoalStatus::Paused
        | AppThreadGoalStatus::Complete
        | AppThreadGoalStatus::Cancelled => goal.objective.clone(),
    };
    Some(GoalStatusIndicator {
        status: goal.status,
        detail,
    })
}
