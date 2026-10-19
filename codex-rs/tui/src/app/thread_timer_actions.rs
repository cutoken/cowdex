use super::App;
use crate::app_server_session::AppServerSession;
use chrono::DateTime;
use chrono::Utc;
use codex_app_server_protocol::ThreadTimer;
use codex_protocol::ThreadId;
use ratatui::style::Stylize;
use ratatui::text::Line;

impl App {
    pub(super) async fn list_thread_timers(
        &mut self,
        app_server: &mut AppServerSession,
        thread_id: ThreadId,
    ) {
        let result = app_server.thread_timer_list(thread_id).await;
        if self.current_displayed_thread_id() != Some(thread_id) {
            return;
        }
        match result {
            Ok(timers) => self
                .chat_widget
                .add_plain_history_lines(timer_list_lines(&timers)),
            Err(err) => {
                tracing::warn!(%err, "failed to list thread timers");
                self.chat_widget
                    .add_error_message("Couldn’t load timers. Try again.".to_string());
            }
        }
    }

    pub(super) async fn clear_thread_timers(
        &mut self,
        app_server: &mut AppServerSession,
        thread_id: ThreadId,
    ) {
        let result = app_server.thread_timer_clear(thread_id).await;
        if self.current_displayed_thread_id() != Some(thread_id) {
            return;
        }
        match result {
            Ok(response) => {
                let message = match response.cleared {
                    0 => "No timers to clear.".to_string(),
                    1 => "Cleared 1 timer.".to_string(),
                    count => format!("Cleared {count} timers."),
                };
                self.chat_widget.add_info_message(message, /*hint*/ None);
            }
            Err(err) => {
                tracing::warn!(%err, "failed to clear thread timers");
                self.chat_widget
                    .add_error_message("Couldn’t clear timers. Try again.".to_string());
            }
        }
    }
}

fn timer_list_lines(timers: &[ThreadTimer]) -> Vec<Line<'static>> {
    if timers.is_empty() {
        return vec!["No timers set.".into()];
    }

    let mut lines = Vec::with_capacity(timers.len() + 1);
    lines.push("Timers".bold().into());
    lines.extend(timers.iter().map(|timer| {
        let reminder = timer.reminder.replace(['\n', '\r'], " ");
        let fires_at = DateTime::<Utc>::from_timestamp(timer.next_fire_at, /*nsecs*/ 0)
            .map(|time| time.format("%-d %b %Y, %H:%M UTC").to_string())
            .unwrap_or_else(|| "Unknown time".to_string());
        let mut line = Line::from(vec![
            "• ".dim(),
            reminder.into(),
            format!(" · {fires_at}").dim(),
        ]);
        if let Some(interval_ms) = timer.interval_ms {
            line.push_span(format!(" · every {}", format_interval(interval_ms)).dim());
        }
        line
    }));
    lines
}

fn format_interval(interval_ms: u64) -> String {
    const SECOND_MS: u64 = 1_000;
    const MINUTE_MS: u64 = 60 * SECOND_MS;
    const HOUR_MS: u64 = 60 * MINUTE_MS;
    const DAY_MS: u64 = 24 * HOUR_MS;

    for (unit_ms, suffix) in [
        (DAY_MS, "d"),
        (HOUR_MS, "h"),
        (MINUTE_MS, "m"),
        (SECOND_MS, "s"),
    ] {
        if interval_ms >= unit_ms && interval_ms.is_multiple_of(unit_ms) {
            return format!("{}{suffix}", interval_ms / unit_ms);
        }
    }
    format!("{interval_ms}ms")
}

#[cfg(test)]
#[path = "thread_timer_actions_tests.rs"]
mod tests;
