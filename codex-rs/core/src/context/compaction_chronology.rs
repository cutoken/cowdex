use chrono::DateTime;
use chrono::Utc;
use codex_history::ResponseItemEnvelope;
use codex_protocol::items::TurnItem;
use codex_protocol::models::ResponseItem;

use super::ContextualUserFragment;

/// Records the age of the last real user message at a compaction checkpoint.
/// The fixed checkpoint time keeps this fragment stable across subsequent requests.
pub(crate) struct CompactionChronology {
    compacted_at: DateTime<Utc>,
    sent_at: Option<DateTime<Utc>>,
}

impl CompactionChronology {
    pub(crate) fn from_history(
        history: &[ResponseItemEnvelope],
        compacted_at: DateTime<Utc>,
    ) -> Option<Self> {
        let user_message = history.iter().rev().find(|envelope| {
            matches!(
                crate::event_mapping::parse_turn_item(&envelope.item),
                Some(TurnItem::UserMessage(user)) if !crate::compact::is_summary_message(&user.message())
            )
        })?;
        let ResponseItem::Message {
            internal_chat_message_metadata_passthrough,
            ..
        } = &user_message.item
        else {
            return None;
        };
        let sent_at = internal_chat_message_metadata_passthrough
            .as_ref()
            .and_then(|metadata| metadata.create_time.as_ref())
            .and_then(serde_json::Number::as_f64)
            .filter(|seconds| seconds.is_finite())
            .and_then(|seconds| DateTime::from_timestamp(seconds as i64, /*nsecs*/ 0));
        Some(Self {
            compacted_at,
            sent_at,
        })
    }
}

impl ContextualUserFragment for CompactionChronology {
    fn role(&self) -> &'static str {
        "developer"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("<compaction_chronology>", "</compaction_chronology>")
    }

    fn body(&self) -> String {
        let checkpoint = self.compacted_at.format("%Y-%m-%d %H:%M:%S UTC");
        let age = match self.sent_at {
            Some(sent_at) => {
                let seconds = (self.compacted_at - sent_at).num_seconds().max(0);
                let hours = seconds / 3600;
                let minutes = seconds % 3600 / 60;
                let seconds = seconds % 60;
                let sent_at = sent_at.format("%Y-%m-%d %H:%M:%S UTC");
                format!(
                    "was sent at {sent_at} ({hours}h {minutes}m {seconds}s before this checkpoint)"
                )
            }
            None => "has an unavailable original send time".to_string(),
        };
        format!(
            "Context was compacted at {checkpoint}. The last real user message before compaction {age}. \
             Retained pre-compaction user messages are historical context, not newly sent instructions. \
             Work may already have addressed them after they were sent. Use the compaction summary and \
             retained results to distinguish completed work from outstanding work; continue from the \
             latest progress rather than repeating completed actions or over-focusing on the last \
             retained user message. Follow any newer user instructions after this checkpoint."
        )
    }
}

#[cfg(test)]
#[path = "compaction_chronology_tests.rs"]
mod tests;
