use super::*;
use crate::AppendThreadItemsParams;
use crate::ResumeThreadParams;
use crate::ThreadPersistenceMetadata;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_protocol::protocol::ThreadMemoryMode;
use pretty_assertions::assert_eq;
use std::fs;

#[tokio::test]
async fn trim_preserves_resume_context_backup_and_subsequent_appends() {
    let home = TempDir::new().unwrap();
    let uuid = Uuid::new_v4();
    let thread_id = ThreadId::from_string(&uuid.to_string()).unwrap();
    let path = write_ordinaled_paginated_rollout(
        home.path(),
        "2025-01-03T13-00-00",
        uuid,
        [
            turn_started("old"),
            user_message(&"obsolete ".repeat(200_000)),
            completed_user_message("old", "old"),
            turn_context(home.path(), "old"),
            compacted("old checkpoint", Some(Vec::new())),
            turn_complete("old"),
            turn_started("latest"),
            user_message("continue work"),
            completed_user_message("latest", "continue work"),
            turn_context(home.path(), "latest"),
            compacted(
                "latest checkpoint",
                Some(vec![ResponseItem::Message {
                    id: None,
                    role: "user".into(),
                    content: vec![ContentItem::InputText {
                        text: "Summary of earlier work".into(),
                    }],
                    phase: None,
                    internal_chat_message_metadata_passthrough: None,
                }]),
            ),
            turn_complete("latest"),
        ],
    );
    let original = fs::read(&path).unwrap();
    let store = LocalThreadStore::new(test_config(home.path()), /*state_db*/ None);
    let params = LoadThreadHistoryParams {
        thread_id,
        include_archived: false,
    };
    let before = serde_json::to_value(
        store
            .load_latest_model_context(params.clone())
            .await
            .unwrap()
            .items,
    )
    .unwrap();
    // A surviving child must keep its immutable byte offsets into the original source.
    let child_id = Uuid::new_v4();
    let child = write_session_file_with_history_mode(
        home.path(),
        "2025-01-03T13-00-01",
        child_id,
        ThreadHistoryMode::Paginated,
    )
    .unwrap();
    let mut child_meta: RolloutLine =
        serde_json::from_str(&fs::read_to_string(&child).unwrap()).unwrap();
    if let RolloutItem::SessionMeta(meta) = &mut child_meta.item {
        meta.meta.history_base = Some(history_position(&path, thread_id, 3));
    }
    fs::write(
        &child,
        format!("{}\n", serde_json::to_string(&child_meta).unwrap()),
    )
    .unwrap();
    let protected = store.trim_rollouts().await.unwrap();
    assert_eq!((protected.trimmed, protected.skipped), (0, 1));
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_file(child).unwrap();
    let writer = store.writer_lock_coordinator.acquire(thread_id).unwrap();
    assert_eq!(store.trim_rollouts().await.unwrap().trimmed, 0);
    assert_eq!(fs::read(&path).unwrap(), original);
    drop(writer);
    let report = store.trim_rollouts().await.unwrap();
    assert_eq!((report.trimmed, report.skipped), (1, 0));
    assert!(fs::metadata(&path).unwrap().len() < 10_000);
    let after = serde_json::to_value(
        store
            .load_latest_model_context(params.clone())
            .await
            .unwrap()
            .items,
    )
    .unwrap();
    assert_eq!(after, before);
    let backup = fs::read_dir(home.path().join("backups/rollouts"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        zstd::stream::decode_all(fs::File::open(backup).unwrap()).unwrap(),
        original
    );
    store
        .resume_thread(ResumeThreadParams {
            thread_id,
            rollout_path: Some(path.clone()),
            history: None,
            include_archived: false,
            metadata: ThreadPersistenceMetadata {
                cwd: Some(home.path().to_path_buf()),
                model_provider: "test-provider".into(),
                memory_mode: ThreadMemoryMode::Enabled,
            },
        })
        .await
        .unwrap();
    store
        .append_items(AppendThreadItemsParams {
            thread_id,
            items: vec![user_message("next input")],
        })
        .await
        .unwrap();
    store.shutdown_thread(thread_id).await.unwrap();
    let context = store.load_latest_model_context(params).await.unwrap().items;
    assert_eq!(
        serde_json::to_value(context.last().unwrap()).unwrap(),
        serde_json::to_value(user_message("next input")).unwrap()
    );
    let lines: Vec<RolloutLine> = fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        lines.iter().map(|line| line.ordinal).collect::<Vec<_>>(),
        (0..lines.len() as u64).map(Some).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn cleanup_protects_fork_sources_live_writers_and_recent_subagents() {
    let home = TempDir::new().unwrap();
    let mut paths = Vec::new();
    for i in 1..=5 {
        let id = Uuid::from_u128(i);
        let path = write_session_file_with_history_mode(
            home.path(),
            "2025-01-03T13-00-00",
            id,
            ThreadHistoryMode::Paginated,
        )
        .unwrap();
        let mut line: RolloutLine =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        if let RolloutItem::SessionMeta(meta) = &mut line.item {
            if i != 1 {
                meta.meta.source = SessionSource::SubAgent(SubAgentSource::Other("test".into()));
            }
            if i == 5 {
                meta.meta.history_base = Some(HistoryPosition {
                    thread_id: ThreadId::from_string(&Uuid::from_u128(3).to_string()).unwrap(),
                    end_ordinal_exclusive: 1,
                    end_byte_offset: 1,
                });
            }
        }
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string(&line).unwrap()),
        )
        .unwrap();
        let time = std::time::SystemTime::now()
            - std::time::Duration::from_secs(if i == 5 { 60 } else { 8 * 24 * 60 * 60 });
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(time))
            .unwrap();
        paths.push(path);
    }
    let store = LocalThreadStore::new(test_config(home.path()), /*state_db*/ None);
    let live_id = ThreadId::from_string(&Uuid::from_u128(4).to_string()).unwrap();
    let _live = store.writer_lock_coordinator.acquire(live_id).unwrap();
    let report = store.cleanup_stale_subagents().await.unwrap();
    assert_eq!((report.removed_sessions, report.kept_referenced), (1, 1));
    assert_eq!(
        paths.iter().map(|path| path.exists()).collect::<Vec<_>>(),
        vec![true, false, true, true, true]
    );
}

#[tokio::test]
async fn trim_real_rollout_fixture_preserves_exact_resume_context() {
    let Some(source) = std::env::var_os("COWDEX_ROLLOUT_FIXTURE") else {
        return;
    };
    let source = PathBuf::from(source);
    let home = TempDir::new().unwrap();
    let directory = home.path().join("sessions/2026/09/15");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(source.file_name().unwrap());
    fs::copy(&source, &path).unwrap();
    let thread_id = codex_rollout::read_session_meta_line(&path)
        .await
        .unwrap()
        .meta
        .id;
    let store = LocalThreadStore::new(test_config(home.path()), /*state_db*/ None);
    let params = LoadThreadHistoryParams {
        thread_id,
        include_archived: false,
    };
    let before = serde_json::to_value(
        store
            .load_latest_model_context(params.clone())
            .await
            .unwrap()
            .items,
    )
    .unwrap();
    let report = store.trim_rollouts().await.unwrap();
    assert_eq!(report.trimmed, 1);
    let after =
        serde_json::to_value(store.load_latest_model_context(params).await.unwrap().items).unwrap();
    assert_eq!(after, before);
    eprintln!(
        "Real rollout: {} bytes -> {} bytes; resume context unchanged",
        fs::metadata(source).unwrap().len(),
        fs::metadata(path).unwrap().len()
    );
}

#[tokio::test]
async fn expired_subagent_batches_remove_children_before_their_source() {
    let home = TempDir::new().unwrap();
    let parent = ThreadId::new();
    let mut parent_end = 0;
    for i in 0..131 {
        let id = if i == 0 { parent } else { ThreadId::new() };
        let path = write_session_file_with_history_mode(
            home.path(),
            "2025-01-03T13-00-00",
            Uuid::parse_str(&id.to_string()).unwrap(),
            ThreadHistoryMode::Paginated,
        )
        .unwrap();
        let mut line: RolloutLine =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        if let RolloutItem::SessionMeta(meta) = &mut line.item {
            meta.meta.source = SessionSource::SubAgent(SubAgentSource::Other("test".into()));
            if i > 0 {
                meta.meta.history_base = Some(HistoryPosition {
                    thread_id: parent,
                    end_ordinal_exclusive: 1,
                    end_byte_offset: parent_end,
                });
            }
        }
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string(&line).unwrap()),
        )
        .unwrap();
        if i == 0 {
            parent_end = fs::metadata(&path).unwrap().len();
        }
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(
                std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 24 * 60 * 60),
            ))
            .unwrap();
    }
    // A reverted thread can also own multiple rollout files referring to each other.
    let replacement = write_session_file_with_history_mode(
        home.path(),
        "2025-01-03T13-00-01",
        Uuid::new_v4(),
        ThreadHistoryMode::Paginated,
    )
    .unwrap();
    let mut line: RolloutLine =
        serde_json::from_str(&fs::read_to_string(&replacement).unwrap()).unwrap();
    line.ordinal = Some(1);
    if let RolloutItem::SessionMeta(meta) = &mut line.item {
        meta.meta.id = parent;
        meta.meta.session_id = parent.into();
        meta.meta.source = SessionSource::SubAgent(SubAgentSource::Other("test".into()));
        meta.meta.history_base = Some(HistoryPosition {
            thread_id: parent,
            end_ordinal_exclusive: 1,
            end_byte_offset: parent_end,
        });
    }
    fs::write(
        &replacement,
        format!("{}\n", serde_json::to_string(&line).unwrap()),
    )
    .unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(replacement)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(
            std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 24 * 60 * 60),
        ))
        .unwrap();
    let store = LocalThreadStore::new(test_config(home.path()), /*state_db*/ None);
    let report = store.cleanup_stale_subagents().await.unwrap();
    assert_eq!((report.removed_sessions, report.kept_referenced), (131, 0));
    assert_eq!(
        codex_rollout::RolloutReferenceIndex::scan(home.path())
            .await
            .unwrap()
            .rollouts()
            .count(),
        0
    );
}
