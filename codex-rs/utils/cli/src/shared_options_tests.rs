use super::*;
use clap::Parser;
use pretty_assertions::assert_eq;

#[derive(Debug, Parser)]
struct TestCli {
    #[command(flatten)]
    shared: SharedCliOptions,

    #[command(flatten)]
    overrides: CliConfigOverrides,
}

#[test]
fn worker_syntax_defaults_and_effort_are_independent_of_the_lead() {
    for (args, lead, lead_effort, worker, worker_effort) in [
        (
            vec!["cowdex", "--worker"],
            "gpt-6-astra",
            ReasoningEffort::Medium,
            "gpt-6.1-sol",
            "medium",
        ),
        (
            vec!["cowdex", "--worker", "sol"],
            "gpt-6-astra",
            ReasoningEffort::Medium,
            "gpt-6.1-sol",
            "medium",
        ),
        (
            vec!["cowdex", "--worker", ":high"],
            "gpt-6-astra",
            ReasoningEffort::Medium,
            "gpt-6.1-sol",
            "high",
        ),
        (
            vec![
                "cowdex",
                "--worker",
                "luna:low",
                "--model",
                "gpt-6.1-sol",
                "--effort",
                "high",
            ],
            "gpt-6.1-sol",
            ReasoningEffort::High,
            "gpt-6-luna",
            "low",
        ),
    ] {
        let mut cli = TestCli::parse_from(args);
        cli.shared.take_config_overrides(&mut cli.overrides);
        let mut expected = expected_preset_overrides(/*enabled_setting*/ None);
        expected.push(format!("features.multi_agent_v2.boss_mode={{enabled=true,model=\"{worker}\",reasoning_effort=\"{worker_effort}\"}}"));
        assert_eq!(
            (
                cli.shared.model.as_deref(),
                cli.shared.ultra_effort,
                cli.overrides.raw_overrides
            ),
            (Some(lead), Some(lead_effort), expected)
        );
    }
    for value in ["sol:extreme", "sol:high:low", ":"] {
        assert!(TestCli::try_parse_from(["cowdex", "--worker", value]).is_err());
    }
}

#[test]
fn effort_controls_solo_ultra_and_external_provider_requests() {
    for effort in ["low", "medium", "high", "xhigh", "max"] {
        let mut solo = TestCli::parse_from(["cowdex", "--effort", effort]);
        solo.shared.take_config_overrides(&mut solo.overrides);
        assert_eq!(
            (
                solo.shared.model,
                solo.shared.ultra_effort,
                solo.overrides.raw_overrides
            ),
            (
                None,
                None,
                vec![format!("model_reasoning_effort=\"{effort}\"")]
            )
        );
        for mode in ["--ultra", "--zai", "--openrouter"] {
            let mut cli = TestCli::parse_from(["cowdex", mode, "--effort", effort]);
            cli.shared.take_config_overrides(&mut cli.overrides);
            if mode == "--ultra" {
                assert_eq!(
                    cli.shared.ultra_effort,
                    Some(parse_ultra_effort(effort).unwrap())
                );
            } else {
                assert_eq!(
                    cli.overrides.raw_overrides.last(),
                    Some(&format!("model_reasoning_effort=\"{effort}\""))
                );
            }
        }
    }
}

#[test]
fn worker_and_main_effort_survive_exec_inheritance_and_resume_overrides() {
    let root = TestCli::parse_from(["cowdex", "--worker", "sol:high", "--effort", "high"]);
    let mut exec = SharedCliOptions::default();
    exec.inherit_exec_root_options(&root.shared);
    let child = TestCli::parse_from(["cowdex", "--worker", "luna:low", "--effort", "medium"]);
    exec.apply_subcommand_overrides(child.shared);
    let mut overrides = CliConfigOverrides::default();
    exec.take_config_overrides(&mut overrides);
    let mut expected = expected_preset_overrides(/*enabled_setting*/ None);
    expected.push("features.multi_agent_v2.boss_mode={enabled=true,model=\"gpt-6-luna\",reasoning_effort=\"low\"}".to_string());
    assert_eq!(
        (exec.ultra_effort, overrides.raw_overrides),
        (Some(ReasoningEffort::Medium), expected)
    );
}

#[test]
fn unified_team_options_preserve_the_lead_and_set_worker_defaults() {
    let mut cli = TestCli::parse_from([
        "cowdex",
        "--ultra",
        "--model",
        "gpt-6.1-sol",
        "--boss",
        "luna",
        "--worker-effort",
        "low",
    ]);
    cli.shared.take_config_overrides(&mut cli.overrides);
    assert_eq!(cli.shared.model.as_deref(), Some("gpt-6.1-sol"));
    assert!(cli.overrides.raw_overrides.contains(&"features.multi_agent_v2.boss_mode={enabled=true,model=\"gpt-6-luna\",reasoning_effort=\"low\"}".to_string()));
    assert!(
        cli.overrides
            .raw_overrides
            .contains(&r#"model_reasoning_effort="ultra""#.to_string())
    );
    let mut cli = TestCli::parse_from(["cowdex", "--boss", "sol"]);
    cli.shared.take_config_overrides(&mut cli.overrides);
    assert_eq!(cli.shared.model.as_deref(), Some("gpt-6-astra"));
    assert!(cli.overrides.raw_overrides.contains(&"features.multi_agent_v2.boss_mode={enabled=true,model=\"gpt-6.1-sol\",reasoning_effort=\"medium\"}".to_string()));
}

#[test]
fn worker_options_merge_across_exec_and_resume() {
    let root = TestCli::parse_from(["cowdex", "--boss", "sol", "--worker-effort", "high"]);
    let mut exec = SharedCliOptions::default();
    exec.inherit_exec_root_options(&root.shared);
    assert_eq!(exec.boss.as_deref(), Some("sol"));
    assert_eq!(exec.worker_effort, Some(ReasoningEffort::High));
    let child = TestCli::parse_from(["cowdex", "--boss", "luna", "--worker-effort", "low"]);
    exec.apply_subcommand_overrides(child.shared);
    assert_eq!(exec.boss.as_deref(), Some("luna"));
    assert_eq!(exec.worker_effort, Some(ReasoningEffort::Low));
    let provider = TestCli::parse_from(["cowdex", "--zai", "--ultra"]);
    exec.apply_subcommand_overrides(provider.shared);
    let mut overrides = CliConfigOverrides::default();
    exec.take_config_overrides(&mut overrides);
    assert_eq!(exec.model.as_deref(), Some("glm-5.3"));
    assert!(
        !overrides
            .raw_overrides
            .iter()
            .any(|value| value.starts_with("features.multi_agent_v2.boss_mode="))
    );
}

fn expected_preset_overrides(enabled_setting: Option<&str>) -> Vec<String> {
    std::iter::once(r#"model_reasoning_effort="ultra""#.to_string())
        .chain(
            ["astra_boss", "astra_ultra_low", "astra_boss_low"].map(|setting| {
                format!(
                    "features.multi_agent_v2.{setting}={}",
                    enabled_setting == Some(setting)
                )
            }),
        )
        .chain(std::iter::once(
            "features.multi_agent_v2.enabled=true".to_string(),
        ))
        .collect()
}

#[test]
fn compact_with_lower_flag_reaches_config_through_command_merges() {
    let parsed = || {
        TestCli::try_parse_from(["cowdex", "--compact-with-lower"])
            .expect("lower compaction arguments should parse")
            .shared
    };
    let mut inherited = SharedCliOptions::default();
    inherited.inherit_exec_root_options(&parsed());
    let mut merged = SharedCliOptions::default();
    merged.apply_subcommand_overrides(parsed());

    for mut shared in [parsed(), inherited, merged] {
        let mut overrides = CliConfigOverrides::default();
        shared.take_config_overrides(&mut overrides);
        shared.take_config_overrides(&mut overrides);
        assert_eq!(overrides.raw_overrides, vec!["compact_with_lower=true"]);
    }
}

#[test]
fn astra_ultra_selects_astra_ultra_medium() {
    let mut cli = TestCli::try_parse_from(["cowdex", "--astra-ultra"])
        .expect("astra ultra arguments should parse");

    cli.shared.take_config_overrides(&mut cli.overrides);

    assert_eq!(cli.shared.model.as_deref(), Some("gpt-6-astra"));
    assert_eq!(cli.shared.ultra_effort, Some(ReasoningEffort::Medium));
    assert_eq!(
        cli.overrides.raw_overrides,
        expected_preset_overrides(/*enabled_setting*/ None)
    );
}

#[test]
fn ultra_effort_selects_ultra_without_replacing_an_explicit_effort_override() {
    let mut ultra = TestCli::try_parse_from(["cowdex", "--ultra-effort", "high"])
        .expect("ultra effort arguments should parse");
    ultra.shared.take_config_overrides(&mut ultra.overrides);

    let mut configured = TestCli::try_parse_from([
        "cowdex",
        "--ultra-effort",
        "high",
        "-c",
        "model_reasoning_effort=low",
    ])
    .expect("ultra effort and config override arguments should parse");
    configured
        .shared
        .take_config_overrides(&mut configured.overrides);

    assert_eq!(
        ultra.overrides.raw_overrides,
        vec![r#"model_reasoning_effort="ultra""#]
    );
    assert_eq!(
        configured.overrides.raw_overrides,
        vec!["model_reasoning_effort=low"]
    );
}

#[test]
fn astra_boss_preserves_explicit_ultra_effort_and_enables_worker_policy() {
    let mut cli = TestCli::try_parse_from(["cowdex", "--astra-boss", "--ultra-effort", "high"])
        .expect("astra boss arguments should parse");

    cli.shared.take_config_overrides(&mut cli.overrides);

    assert_eq!(cli.shared.model.as_deref(), Some("gpt-6-astra"));
    assert_eq!(cli.shared.ultra_effort, Some(ReasoningEffort::High));
    assert_eq!(
        cli.overrides.raw_overrides,
        expected_preset_overrides(Some("astra_boss"))
    );
}

#[test]
fn astra_fixed_worker_presets_select_main_effort_and_enable_worker_policy() {
    for (flag, main_effort, enabled_setting) in [
        (
            "--astra-ultra-low",
            ReasoningEffort::Medium,
            "astra_ultra_low",
        ),
        (
            "--astra-boss-low",
            ReasoningEffort::Medium,
            "astra_boss_low",
        ),
        (
            "--astra-ultra-miser",
            ReasoningEffort::Low,
            "astra_ultra_low",
        ),
        ("--astra-boss-miser", ReasoningEffort::Low, "astra_boss_low"),
    ] {
        let mut cli = TestCli::try_parse_from(["cowdex", flag])
            .expect("Astra low preset arguments should parse");

        cli.shared.take_config_overrides(&mut cli.overrides);

        assert_eq!(cli.shared.model.as_deref(), Some("gpt-6-astra"));
        assert_eq!(cli.shared.ultra_effort, Some(main_effort));
        assert_eq!(
            cli.overrides.raw_overrides,
            expected_preset_overrides(Some(enabled_setting))
        );
    }
}

#[test]
fn astra_presets_reject_conflicting_model_or_each_other() {
    for args in [
        vec!["cowdex", "--astra-ultra", "--astra-boss"],
        vec!["cowdex", "--astra-boss", "--model", "gpt-5.6-sol"],
        vec!["cowdex", "--astra-ultra-low", "--astra-boss-low"],
        vec!["cowdex", "--astra-ultra-low", "--astra-ultra"],
        vec!["cowdex", "--astra-boss-low", "--astra-boss"],
        vec!["cowdex", "--astra-ultra-miser", "--astra-boss-miser"],
        vec!["cowdex", "--astra-ultra-miser", "--ultra-effort", "high"],
        vec!["cowdex", "--astra-boss-miser", "--ultra-effort", "high"],
    ] {
        assert!(TestCli::try_parse_from(args).is_err());
    }
}

#[test]
fn external_provider_flags_select_their_provider_and_model() {
    for (flag, provider_id, model, base_url, env_key) in [
        (
            "--zai",
            "zai",
            "glm-5.3",
            "https://api.z.ai/api/v1",
            "ZAI_API_KEY",
        ),
        (
            "--openrouter",
            "openrouter",
            "deepseek/deepseek-v4.1-flash",
            "https://openrouter.ai/api/v1",
            "OPENROUTER_API_KEY",
        ),
    ] {
        let mut cli =
            TestCli::try_parse_from(["cowdex", flag]).expect("provider arguments should parse");
        cli.shared.take_config_overrides(&mut cli.overrides);
        let parsed = cli.overrides.parse_overrides().expect("valid overrides");
        let provider = parsed
            .iter()
            .find_map(|(key, value)| {
                (key == &format!("model_providers.{provider_id}")).then_some(value)
            })
            .and_then(toml::Value::as_table)
            .expect("provider config override");
        let auth = provider
            .get("auth")
            .and_then(toml::Value::as_table)
            .expect("provider auth config");
        let args = auth
            .get("args")
            .and_then(toml::Value::as_array)
            .expect("provider auth args");

        assert_eq!(cli.shared.model.as_deref(), Some(model));
        assert_eq!(
            parsed
                .iter()
                .find_map(|(key, value)| (key == "model_provider").then_some(value)),
            Some(&toml::Value::String(provider_id.to_string()))
        );
        assert_eq!(
            parsed.iter().find_map(|(key, value)| {
                (key == "features.standalone_web_search").then_some(value)
            }),
            Some(&toml::Value::Boolean(true))
        );
        assert_eq!(
            provider.get("base_url").and_then(toml::Value::as_str),
            Some(base_url)
        );
        assert_eq!(
            args.iter()
                .filter_map(toml::Value::as_str)
                .collect::<Vec<_>>(),
            vec!["internal-provider-credential", provider_id, env_key]
        );
    }
}

#[test]
fn external_provider_flags_conflict_with_other_model_selections() {
    for args in [
        vec!["cowdex", "--zai", "--openrouter"],
        vec!["cowdex", "--zai", "--model", "gpt-6-astra"],
        vec!["cowdex", "--openrouter", "--astra-boss"],
    ] {
        assert!(TestCli::try_parse_from(args).is_err());
    }
}

#[test]
fn external_providers_support_ultra_mode_without_changing_model() {
    for (provider_flag, model, effort) in [
        ("--zai", "glm-5.3", ReasoningEffort::Max),
        (
            "--openrouter",
            "deepseek/deepseek-v4.1-flash",
            ReasoningEffort::High,
        ),
    ] {
        let mut cli = TestCli::try_parse_from(["cowdex", provider_flag, "--astra-ultra"])
            .expect("external provider should support ultra mode");

        cli.shared.take_config_overrides(&mut cli.overrides);

        assert_eq!(cli.shared.model.as_deref(), Some(model));
        assert_eq!(cli.shared.ultra_effort, Some(effort));
        assert!(
            cli.overrides
                .raw_overrides
                .iter()
                .any(|value| value == "features.multi_agent_v2.enabled=true")
        );
        assert!(
            cli.overrides
                .raw_overrides
                .iter()
                .any(|value| value == r#"model_reasoning_effort="ultra""#)
        );
    }
}

#[test]
fn exec_subcommands_inherit_astra_fixed_worker_presets_from_the_root_command() {
    for (flag, main_effort, enabled_setting) in [
        (
            "--astra-ultra-low",
            ReasoningEffort::Medium,
            "astra_ultra_low",
        ),
        (
            "--astra-boss-low",
            ReasoningEffort::Medium,
            "astra_boss_low",
        ),
        (
            "--astra-ultra-miser",
            ReasoningEffort::Low,
            "astra_ultra_low",
        ),
        ("--astra-boss-miser", ReasoningEffort::Low, "astra_boss_low"),
    ] {
        let root = TestCli::try_parse_from(["cowdex", flag])
            .expect("root Astra low arguments should parse")
            .shared;
        let mut subcommand = SharedCliOptions::default();
        let mut overrides = CliConfigOverrides::default();

        subcommand.inherit_exec_root_options(&root);
        subcommand.take_config_overrides(&mut overrides);

        assert_eq!(subcommand.model.as_deref(), Some("gpt-6-astra"));
        assert_eq!(subcommand.ultra_effort, Some(main_effort));
        assert_eq!(
            overrides.raw_overrides,
            expected_preset_overrides(Some(enabled_setting))
        );
    }
}

#[test]
fn exec_subcommands_inherit_astra_ultra_from_the_root_command() {
    let root = TestCli::try_parse_from(["cowdex", "--astra-ultra"])
        .expect("root Astra arguments should parse")
        .shared;
    let mut subcommand = SharedCliOptions::default();
    let mut overrides = CliConfigOverrides::default();

    subcommand.inherit_exec_root_options(&root);
    subcommand.take_config_overrides(&mut overrides);

    assert_eq!(subcommand.model.as_deref(), Some("gpt-6-astra"));
    assert_eq!(subcommand.ultra_effort, Some(ReasoningEffort::Medium));
    assert_eq!(
        overrides.raw_overrides,
        expected_preset_overrides(/*enabled_setting*/ None)
    );
}
