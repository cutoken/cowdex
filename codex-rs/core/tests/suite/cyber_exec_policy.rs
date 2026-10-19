use anyhow::Result;
use codex_core::TurnInputRequest;
use codex_core::config::Config;
use codex_core::config::Constrained;
use codex_core::sandboxing::SandboxPermissions;
use codex_features::Feature;
use codex_protocol::config_types::ApprovalsReviewer;
use codex_protocol::openai_models::MODEL_SPECIALTY_CYBER;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ReviewDecision;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::protocol::ThreadSettingsOverrides;
use codex_protocol::user_input::UserInput;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_function_call;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse;
use core_test_support::responses::sse_completed;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::skip_if_sandbox;
use core_test_support::skip_if_wine_exec;
use core_test_support::submit_thread_settings;
use core_test_support::test_codex::TestCodex;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::fs;
use test_case::test_case;

const TEST_COMMAND: &str = "git version";
const SAVED_PREFIX: &str = r#"["git", "version"]"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommandTool {
    ShellCommand,
    UnifiedExec,
}

impl CommandTool {
    fn configure(self, config: &mut Config) {
        if self == Self::UnifiedExec {
            config.use_experimental_unified_exec_tool = true;
            config
                .features
                .enable(Feature::UnifiedExec)
                .expect("enable unified exec");
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModelSpecialty {
    Cyber,
    General,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellBackend {
    Standard,
    ZshFork,
}

fn configure_saved_prefix_and_guardian(config: &mut Config) {
    let policy_path = config.codex_home.join("rules/default.rules");
    fs::create_dir_all(policy_path.parent().expect("rules directory"))
        .expect("create rules directory");
    fs::write(
        policy_path,
        r#"prefix_rule(pattern=["git", "version"], decision="allow")"#,
    )
    .expect("write saved command prefix rule");
    config.permissions.approval_policy = Constrained::allow_any(AskForApproval::OnRequest);
    config.approvals_reviewer = ApprovalsReviewer::AutoReview;
    config
        .set_legacy_sandbox_policy(SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        })
        .expect("set workspace sandbox policy");
}

fn command_response(response_id: &str, call_id: &str, command_tool: CommandTool) -> Result<String> {
    let (tool_name, command_key, timeout_key) = match command_tool {
        CommandTool::ShellCommand => ("shell_command", "command", "timeout_ms"),
        CommandTool::UnifiedExec => ("exec_command", "cmd", "yield_time_ms"),
    };
    let mut args = json!({
        "sandbox_permissions": SandboxPermissions::RequireEscalated,
        "justification": "Check whether a saved prefix bypasses Guardian.",
        "prefix_rule": ["git", "version"],
    });
    args[command_key] = json!(TEST_COMMAND);
    args[timeout_key] = json!(1_000);

    Ok(sse(vec![
        ev_response_created(response_id),
        ev_function_call(call_id, tool_name, &serde_json::to_string(&args)?),
        ev_completed(response_id),
    ]))
}

async fn submit_model_turn(test: &TestCodex, model: &str, prompt: &str) -> Result<()> {
    submit_thread_settings(
        &test.codex,
        ThreadSettingsOverrides {
            model: Some(model.to_string()),
            approval_policy: Some(AskForApproval::OnRequest),
            approvals_reviewer: Some(ApprovalsReviewer::AutoReview),
            ..Default::default()
        },
    )
    .await?;
    test.submit_text_turn(prompt).await
}

#[test_case(CommandTool::ShellCommand; "shell command")]
#[test_case(CommandTool::UnifiedExec; "unified exec")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cyber_model_user_approval_never_offers_a_reusable_prefix(
    command_tool: CommandTool,
) -> Result<()> {
    skip_if_no_network!(Ok(()));
    skip_if_sandbox!(Ok(()));
    skip_if_wine_exec!(Ok(()), "command approval requires host-native paths");

    let server = start_mock_server().await;
    let mut builder = test_codex()
        .with_model_info_override("gpt-5.4", |model| {
            model.model_specialty = Some(MODEL_SPECIALTY_CYBER.to_string());
        })
        .with_config(move |config| {
            configure_saved_prefix_and_guardian(config);
            command_tool.configure(config);
            config.approvals_reviewer = ApprovalsReviewer::User;
        });
    let test = builder.build_with_auto_env(&server).await?;
    let policy_path = test.codex_home_path().join("rules/default.rules");
    let initial_policy = fs::read_to_string(&policy_path)?;
    let responses = mount_sse_sequence(
        &server,
        vec![
            command_response(
                "parent-one-time-approval",
                "one-time-approval",
                command_tool,
            )?,
            sse_completed("parent-one-time-complete"),
        ],
    )
    .await;

    test.codex
        .start_or_steer_turn(
            TurnInputRequest::user_input(vec![UserInput::Text {
                text: "run a command with one-time approval".to_string(),
                text_elements: Vec::new(),
            }])
            .with_thread_settings(ThreadSettingsOverrides {
                approval_policy: Some(AskForApproval::OnRequest),
                approvals_reviewer: Some(ApprovalsReviewer::User),
                ..Default::default()
            }),
        )
        .await?;

    let EventMsg::ExecApprovalRequest(approval) = wait_for_event(&test.codex, |event| {
        matches!(
            event,
            EventMsg::ExecApprovalRequest(_) | EventMsg::TurnComplete(_)
        )
    })
    .await
    else {
        panic!("expected a one-time command approval request");
    };
    assert_eq!(approval.proposed_execpolicy_amendment, None);
    assert_eq!(
        approval.effective_available_decisions(),
        vec![ReviewDecision::Approved, ReviewDecision::Abort],
    );
    test.codex
        .submit(Op::ExecApproval {
            id: approval.effective_approval_id(),
            turn_id: None,
            decision: ReviewDecision::Approved,
        })
        .await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;

    assert!(
        responses
            .function_call_output_text("one-time-approval")
            .is_some_and(|output| output.contains(TEST_COMMAND)),
    );
    assert_eq!(fs::read_to_string(policy_path)?, initial_policy);
    assert_eq!(responses.requests().len(), 2);

    Ok(())
}
