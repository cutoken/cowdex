use super::*;
use crate::chatwidget::boss_mode::parse_boss_command;
use codex_protocol::protocol::BossModeSettings;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn boss_commands_validate_and_preserve_selection() {
    let (chat, _rx, _ops) = make_chatwidget_manual(/*model_override*/ None).await;
    let mut model = chat.model_catalog.try_list_models().unwrap()[0].clone();
    model.model = "gpt-6.1-sol".to_string();
    model.supported_reasoning_efforts = [
        ReasoningEffortConfig::Medium,
        ReasoningEffortConfig::High,
        ReasoningEffortConfig::Max,
    ]
    .into_iter()
    .map(
        |effort| codex_protocol::openai_models::ReasoningEffortPreset {
            effort,
            description: String::new(),
        },
    )
    .collect();
    let models: Vec<_> = ["gpt-6-astra", "gpt-6.1-sol", "gpt-5.6-terra", "gpt-6-luna"]
        .into_iter()
        .map(|name| ModelPreset {
            model: name.to_string(),
            ..model.clone()
        })
        .collect();
    let enabled = BossModeSettings {
        enabled: true,
        model: "gpt-6.1-sol".to_string(),
        reasoning_effort: ReasoningEffortConfig::Medium,
    };
    for command in ["on", "sol", "sol medium", "gpt-6.1-sol medium"] {
        assert_eq!(
            parse_boss_command(command, /*current*/ None, &models),
            Ok(Some(enabled.clone()))
        );
    }
    let disabled = BossModeSettings {
        enabled: false,
        ..enabled.clone()
    };
    assert_eq!(
        parse_boss_command("off", Some(&enabled), &models),
        Ok(Some(disabled.clone()))
    );
    assert_eq!(
        parse_boss_command("on", Some(&disabled), &models),
        Ok(Some(enabled))
    );
    for (command, model, effort) in [
        ("sol high", "gpt-6.1-sol", ReasoningEffortConfig::High),
        ("luna max", "gpt-6-luna", ReasoningEffortConfig::Max),
        ("astra medium", "gpt-6-astra", ReasoningEffortConfig::Medium),
        ("terra high", "gpt-5.6-terra", ReasoningEffortConfig::High),
    ] {
        assert_eq!(
            parse_boss_command(command, /*current*/ None, &models),
            Ok(Some(BossModeSettings {
                enabled: true,
                model: model.to_string(),
                reasoning_effort: effort
            }))
        );
    }
    for command in ["sol low", "unknown medium", "on extra", "sol medium extra"] {
        assert!(parse_boss_command(command, /*current*/ None, &models).is_err());
    }
}

#[tokio::test]
async fn boss_request_waits_for_authoritative_settings_while_working() {
    let (mut chat, mut rx, _ops) = make_chatwidget_manual(/*model_override*/ None).await;
    let thread_id = ThreadId::new();
    chat.thread_id = Some(thread_id);
    chat.bottom_pane.set_task_running(/*running*/ true);
    chat.dispatch_command_with_args(SlashCommand::Boss, "off".to_string(), Vec::new());
    let settings = std::iter::from_fn(|| rx.try_recv().ok())
        .find_map(|event| match event {
            AppEvent::UpdateBossMode {
                thread_id: target,
                settings,
            } => {
                assert_eq!(target, thread_id);
                Some(settings)
            }
            _ => None,
        })
        .expect("boss settings request");
    assert!(!settings.enabled);
    assert_eq!(chat.config.multi_agent_v2.boss_mode, None);
}

#[tokio::test]
async fn boss_status_replaces_ultra_label() {
    let (mut chat, _rx, _ops) = make_chatwidget_manual(Some("gpt-5.2")).await;
    chat.set_reasoning_effort(Some(ReasoningEffortConfig::High));
    chat.config.multi_agent_v2.boss_mode = Some(BossModeSettings {
        enabled: true,
        model: "gpt-6-luna".to_string(),
        reasoning_effort: ReasoningEffortConfig::Max,
    });
    insta::assert_snapshot!(chat.status_line_value_for_item(StatusLineItem::Reasoning).unwrap(), @"Boss luna max");
    insta::assert_snapshot!(chat.status_line_value_for_item(StatusLineItem::ModelWithReasoning).unwrap(), @"gpt-5.2 high · Boss luna max");
    chat.config.tui_status_line = Some(vec![
        "model-with-reasoning".to_string(),
        "context-remaining".to_string(),
    ]);
    chat.refresh_status_surfaces();
    let width = 60;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(
        width,
        chat.desired_height(width),
    ))
    .unwrap();
    terminal
        .draw(|frame| chat.render(frame.area(), frame.buffer_mut()))
        .unwrap();
    insta::assert_snapshot!(
        "boss_status_footer",
        normalized_backend_snapshot(terminal.backend())
    );
}

#[tokio::test]
async fn boss_usage_snapshot() {
    let (mut chat, mut rx, _ops) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.dispatch_command(SlashCommand::Boss);
    let cells = drain_insert_history(&mut rx);
    insta::assert_snapshot!(lines_to_single_string(&cells[0]));
}
