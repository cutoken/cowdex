use super::*;
use clap::CommandFactory;
use pretty_assertions::assert_eq;

#[test]
fn help_shows_consolidated_worker_and_effort_options() {
    insta::assert_snapshot!(
        MultitoolCli::command()
            .term_width(100)
            .render_help()
            .to_string()
    );
}

#[test]
fn resume_and_fork_accept_worker_defaults_and_explicit_lead_effort() {
    for command in ["resume", "fork"] {
        for worker_args in [vec!["--worker"], vec!["--worker", "sol:medium"]] {
            let mut args = vec!["cowdex", command, "--last", "--effort", "high"];
            args.extend(worker_args);
            let cli = MultitoolCli::parse_from(args);
            let mut shared = match cli.subcommand.expect("subcommand") {
                Subcommand::Resume(cmd) => cmd.config_overrides.0.shared.into_inner(),
                Subcommand::Fork(cmd) => cmd.config_overrides.0.shared.into_inner(),
                _ => unreachable!("resume or fork"),
            };
            let mut overrides = CliConfigOverrides::default();
            shared.take_config_overrides(&mut overrides);
            assert_eq!(
                (shared.model.as_deref(), shared.ultra_effort),
                (
                    Some("gpt-6-astra"),
                    Some(codex_protocol::openai_models::ReasoningEffort::High)
                )
            );
            assert!(overrides.raw_overrides.iter().any(|value| value == "features.multi_agent_v2.boss_mode={enabled=true,model=\"gpt-6.1-sol\",reasoning_effort=\"medium\"}"));
        }
    }
}

#[test]
fn bare_worker_before_a_subcommand_keeps_its_default() {
    for command in ["resume", "fork", "exec"] {
        let cli = MultitoolCli::parse_from(["cowdex", "--worker", command]);
        assert_eq!(cli.interactive.boss.as_deref(), Some("sol"));
        assert!(cli.subcommand.is_some());
    }
    let cli = MultitoolCli::parse_from(["cowdex", "exec", "--worker", "resume", "--last"]);
    let Subcommand::Exec(exec) = cli.subcommand.unwrap() else {
        unreachable!("exec");
    };
    assert_eq!(exec.boss.as_deref(), Some("sol"));
    assert!(exec.command.is_some());
}

#[test]
fn resume_and_fork_accept_provider_qualified_workers() {
    for command in ["resume", "fork"] {
        for worker in ["zai:glm-5.3:max", "openrouter:deepseek/model:free:high"] {
            let cli = MultitoolCli::parse_from(["cowdex", command, "--last", "--worker", worker]);
            let shared = match cli.subcommand.unwrap() {
                Subcommand::Resume(cmd) => cmd.config_overrides.0.shared.into_inner(),
                Subcommand::Fork(cmd) => cmd.config_overrides.0.shared.into_inner(),
                _ => unreachable!(),
            };
            assert_eq!(shared.boss.as_deref(), Some(worker));
        }
    }
}
