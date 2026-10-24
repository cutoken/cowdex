//! TUI token usage models and display formatting.

use std::collections::HashMap;
use std::fmt;

use chrono::Local;
use chrono::NaiveDate;
use chrono::Timelike;
use codex_protocol::num_format::format_with_separators;
use serde::Deserialize;
use serde::Serialize;

const BASELINE_TOKENS: i64 = 12000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
    pub total_tokens: i64,
}

impl TokenUsage {
    pub fn is_zero(&self) -> bool {
        self.total_tokens == 0
    }

    pub(crate) fn cached_input(&self) -> i64 {
        self.cached_input_tokens.max(0)
    }

    pub(crate) fn non_cached_input(&self) -> i64 {
        (self.input_tokens - self.cached_input()).max(0)
    }

    pub(crate) fn blended_total(&self) -> i64 {
        (self.non_cached_input() + self.output_tokens.max(0)).max(0)
    }

    /// Returns the raw `total_tokens` value. For `last_token_usage`, this is the latest active
    /// context size; for `total_token_usage`, this is the accumulated session total.
    pub(crate) fn tokens_in_context_window(&self) -> i64 {
        self.total_tokens
    }

    pub(crate) fn percent_of_context_window_remaining(&self, context_window: i64) -> i64 {
        if context_window <= BASELINE_TOKENS {
            return 0;
        }
        let effective_window = context_window - BASELINE_TOKENS;
        let used = (self.tokens_in_context_window() - BASELINE_TOKENS).max(0);
        let remaining = (effective_window - used).max(0);
        ((remaining as f64 / effective_window as f64) * 100.0)
            .clamp(0.0, 100.0)
            .round() as i64
    }
}

#[derive(Debug)]
pub(crate) struct DailyTokenUsage {
    bucket: NaiveDate,
    active_turns: HashMap<String, String>,
    latest_by_thread: HashMap<String, TokenUsage>,
    total: TokenUsage,
}

impl Default for DailyTokenUsage {
    fn default() -> Self {
        Self {
            bucket: current_daily_bucket(),
            active_turns: HashMap::new(),
            latest_by_thread: HashMap::new(),
            total: TokenUsage::default(),
        }
    }
}

impl DailyTokenUsage {
    pub(crate) fn start_turn(&mut self, thread_id: &str, turn_id: &str) {
        self.active_turns
            .insert(thread_id.to_string(), turn_id.to_string());
    }

    pub(crate) fn observe(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        current: TokenUsage,
        last: TokenUsage,
    ) {
        let previous = self
            .latest_by_thread
            .insert(thread_id.to_string(), current.clone());
        if self.active_turns.get(thread_id).map(String::as_str) != Some(turn_id) {
            return;
        }
        let usage = previous.map_or(last, |previous| TokenUsage {
            input_tokens: current.input_tokens.saturating_sub(previous.input_tokens),
            cached_input_tokens: current
                .cached_input_tokens
                .saturating_sub(previous.cached_input_tokens),
            output_tokens: current.output_tokens.saturating_sub(previous.output_tokens),
            reasoning_output_tokens: current
                .reasoning_output_tokens
                .saturating_sub(previous.reasoning_output_tokens),
            total_tokens: current.total_tokens.saturating_sub(previous.total_tokens),
        });
        self.total.input_tokens += usage.input_tokens.max(0);
        self.total.cached_input_tokens += usage.cached_input_tokens.max(0);
        self.total.output_tokens += usage.output_tokens.max(0);
        self.total.reasoning_output_tokens += usage.reasoning_output_tokens.max(0);
        self.total.total_tokens += usage.total_tokens.max(0);
    }

    pub(crate) fn rollover_on_submission(&mut self) -> bool {
        let bucket = current_daily_bucket();
        if bucket == self.bucket {
            return false;
        }
        self.bucket = bucket;
        // Running turns can continue after a steer crosses the daily boundary.
        // Keep their identities and cumulative baselines; reset only today's totals.
        self.total = TokenUsage::default();
        true
    }

    pub(crate) fn total(&self) -> TokenUsage {
        self.total.clone()
    }
}

fn current_daily_bucket() -> NaiveDate {
    let now = Local::now();
    if now.hour() < 8 {
        now.date_naive()
            .pred_opt()
            .unwrap_or_else(|| now.date_naive())
    } else {
        now.date_naive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TokenUsageInfo {
    pub(crate) total_token_usage: TokenUsage,
    pub(crate) last_token_usage: TokenUsage,
    pub(crate) model_context_window: Option<i64>,
}

impl fmt::Display for TokenUsage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Token usage: total={} input={}{} output={}{}",
            format_with_separators(self.blended_total()),
            format_with_separators(self.non_cached_input()),
            if self.cached_input() > 0 {
                format!(
                    " (+ {} cached)",
                    format_with_separators(self.cached_input())
                )
            } else {
                String::new()
            },
            format_with_separators(self.output_tokens),
            if self.reasoning_output_tokens > 0 {
                format!(
                    " (reasoning {})",
                    format_with_separators(self.reasoning_output_tokens)
                )
            } else {
                String::new()
            }
        )
    }
}

#[cfg(test)]
#[path = "token_usage_tests.rs"]
mod tests;
