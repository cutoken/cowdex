use super::*;
use codex_protocol::models::ContentItem;
use codex_protocol::models::InternalChatMessageMetadataPassthrough;
use pretty_assertions::assert_eq;

fn user_message(text: &str, sent_at: Option<i64>) -> ResponseItemEnvelope {
    ResponseItemEnvelope::new(ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: sent_at.map(|seconds| {
            InternalChatMessageMetadataPassthrough {
                create_time: Some(seconds.into()),
                ..Default::default()
            }
        }),
    })
}

#[test]
fn compaction_chronology_uses_last_real_user_time_and_fixed_checkpoint() {
    let now = DateTime::from_timestamp(1_800_000_000, /*nsecs*/ 0).unwrap();
    let history = vec![
        user_message("older request", Some(now.timestamp() - 30_000)),
        user_message("latest request", Some(now.timestamp() - 3723)),
        user_message(
            &format!("{}\nsummary", crate::compact::SUMMARY_PREFIX),
            Some(now.timestamp()),
        ),
    ];
    let chronology = CompactionChronology::from_history(&history, now).unwrap();
    assert_eq!(
        chronology.sent_at.unwrap().timestamp(),
        now.timestamp() - 3723
    );
    assert!(
        chronology
            .body()
            .contains("1h 2m 3s before this checkpoint")
    );
}

#[test]
fn compaction_chronology_does_not_invent_missing_send_time() {
    let now = DateTime::from_timestamp(1_800_000_000, /*nsecs*/ 0).unwrap();
    let history = vec![user_message("old rollout without timestamps", None)];
    let chronology = CompactionChronology::from_history(&history, now).unwrap();
    assert_eq!(chronology.sent_at, None);
    assert!(chronology.body().contains("unavailable original send time"));
    assert!(CompactionChronology::from_history(&[], now).is_none());
}
