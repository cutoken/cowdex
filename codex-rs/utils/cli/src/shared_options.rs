//! Shared command-line flags used by both interactive and non-interactive Codex entry points.

use crate::CliConfigOverrides;
use crate::SandboxModeCliArg;
use clap::Args;
use codex_protocol::config_types::ProfileV2Name;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::worker_model::WorkerModel;
use std::path::PathBuf;

fn parse_ultra_effort(value: &str) -> Result<ReasoningEffort, String> {
    match value {
        "low" => Ok(ReasoningEffort::Low),
        "medium" => Ok(ReasoningEffort::Medium),
        "high" => Ok(ReasoningEffort::High),
        "xhigh" => Ok(ReasoningEffort::XHigh),
        "max" => Ok(ReasoningEffort::Max),
        _ => Err("expected one of: low, medium, high, xhigh, max".to_string()),
    }
}

fn parse_worker(value: &str) -> Result<String, String> {
    WorkerModel::parse(value)?;
    Ok(value.to_string())
}

#[derive(Args, Clone, Debug, Default)]
pub struct SharedCliOptions {
    /// Optional image(s) to attach to the initial prompt.
    #[arg(
        long = "image",
        short = 'i',
        value_name = "FILE",
        value_delimiter = ',',
        num_args = 1..
    )]
    pub images: Vec<PathBuf>,

    /// Model the agent should use.
    #[arg(long, short = 'm')]
    pub model: Option<String>,

    /// Main agent reasoning effort: low, medium, high, xhigh, or max.
    #[arg(long, value_name = "LEVEL", value_parser = parse_ultra_effort)]
    pub effort: Option<ReasoningEffort>,

    /// Enable agent teams with the selected model (defaults to GPT-6 Astra).
    #[arg(
        long = "ultra",
        alias = "astra-ultra",
        default_value_t = false,
        conflicts_with_all = ["astra_ultra_low", "astra_ultra_miser", "astra_boss", "astra_boss_low", "astra_boss_miser", "oss"]
    )]
    pub astra_ultra: bool,

    /// Fix workers to [PROVIDER:]MODEL[:EFFORT] (default sol:medium). Change live with /worker.
    #[arg(long = "worker", alias = "boss", value_name = "MODEL", num_args = 0..=1, default_missing_value = "sol", value_parser = parse_worker, conflicts_with_all = ["oss", "astra_ultra_low", "astra_ultra_miser", "astra_boss", "astra_boss_low", "astra_boss_miser"])]
    pub boss: Option<String>,

    /// Worker reasoning effort in Boss mode (defaults to medium).
    #[arg(long, hide = true, value_name = "EFFORT", requires = "boss", value_parser = parse_ultra_effort)]
    pub worker_effort: Option<ReasoningEffort>,

    /// Start Astra in Ultra mode and force spawned workers to Astra at low effort.
    #[arg(
        long = "astra-ultra-low",
        hide = true,
        default_value_t = false,
        conflicts_with_all = ["model", "astra_ultra", "astra_ultra_miser", "astra_boss", "astra_boss_low", "astra_boss_miser", "zai", "openrouter", "oss"]
    )]
    pub astra_ultra_low: bool,

    /// Start Astra at low effort and force spawned workers to Astra at low effort.
    #[arg(
        long = "astra-ultra-miser",
        hide = true,
        default_value_t = false,
        conflicts_with_all = ["model", "astra_ultra", "astra_ultra_low", "astra_boss", "astra_boss_low", "astra_boss_miser", "ultra_effort", "zai", "openrouter", "oss"]
    )]
    pub astra_ultra_miser: bool,

    /// Start Astra in Ultra mode and force spawned workers to GPT-5.6 Sol.
    #[arg(
        long = "astra-boss",
        hide = true,
        default_value_t = false,
        conflicts_with_all = ["model", "astra_ultra", "astra_ultra_low", "astra_ultra_miser", "astra_boss_low", "astra_boss_miser", "zai", "openrouter", "oss"]
    )]
    pub astra_boss: bool,

    /// Start Astra in Ultra mode and force spawned workers to GPT-5.6 Sol at medium effort.
    #[arg(
        long = "astra-boss-low",
        hide = true,
        default_value_t = false,
        conflicts_with_all = ["model", "astra_ultra", "astra_ultra_low", "astra_ultra_miser", "astra_boss", "astra_boss_miser", "zai", "openrouter", "oss"]
    )]
    pub astra_boss_low: bool,

    /// Start Astra at low effort and force spawned workers to GPT-5.6 Sol at medium effort.
    #[arg(
        long = "astra-boss-miser",
        hide = true,
        default_value_t = false,
        conflicts_with_all = ["model", "astra_ultra", "astra_ultra_low", "astra_ultra_miser", "astra_boss", "astra_boss_low", "ultra_effort", "zai", "openrouter", "oss"]
    )]
    pub astra_boss_miser: bool,

    /// Reasoning effort sent to the API when Ultra mode is selected.
    #[arg(
        long = "ultra-effort",
        hide = true,
        value_name = "EFFORT",
        value_parser = parse_ultra_effort
    )]
    pub ultra_effort: Option<ReasoningEffort>,

    /// Use the model's maximum context window for the main agent.
    #[arg(long = "max-context", default_value_t = false)]
    pub max_context: bool,

    /// Compact Astra and Sol high-or-higher sessions with Sol at medium effort.
    #[arg(long = "compact-with-lower", default_value_t = false)]
    pub compact_with_lower: bool,

    /// Use the Z.ai Coding Plan with GLM-5.3.
    #[arg(
        long = "zai",
        default_value_t = false,
        conflicts_with_all = ["model", "astra_ultra_low", "astra_ultra_miser", "astra_boss", "astra_boss_low", "astra_boss_miser", "openrouter", "oss"]
    )]
    pub zai: bool,

    /// Use DeepSeek V4.1 Flash through OpenRouter.
    #[arg(
        long = "openrouter",
        default_value_t = false,
        conflicts_with_all = ["model", "astra_ultra_low", "astra_ultra_miser", "astra_boss", "astra_boss_low", "astra_boss_miser", "zai", "oss"]
    )]
    pub openrouter: bool,

    /// Use open-source provider.
    #[arg(long = "oss", default_value_t = false)]
    pub oss: bool,

    /// Specify which local provider to use (lmstudio or ollama).
    /// If not specified with --oss, will use config default or show selection.
    #[arg(long = "local-provider")]
    pub oss_provider: Option<String>,

    /// Layer $CODEX_HOME/<name>.config.toml on top of the base user config.
    #[arg(long = "profile", short = 'p')]
    pub config_profile_v2: Option<ProfileV2Name>,

    /// Select the sandbox policy to use when executing model-generated shell
    /// commands.
    #[arg(long = "sandbox", short = 's')]
    pub sandbox_mode: Option<SandboxModeCliArg>,

    /// Route approval requests through automatic review using the workspace-write sandbox.
    #[arg(
        long = "approve-for-me",
        alias = "not-so-yolo",
        default_value_t = false,
        conflicts_with_all = ["sandbox_mode", "dangerously_bypass_approvals_and_sandbox"]
    )]
    pub auto_review: bool,

    /// Skip all confirmation prompts and execute commands without sandboxing.
    /// EXTREMELY DANGEROUS. Intended solely for running in environments that are externally sandboxed.
    #[arg(
        long = "dangerously-bypass-approvals-and-sandbox",
        alias = "yolo",
        default_value_t = false
    )]
    pub dangerously_bypass_approvals_and_sandbox: bool,

    /// Run enabled hooks without requiring persisted hook trust for this invocation.
    /// DANGEROUS. Intended only for automation that already vets hook sources.
    #[arg(long = "dangerously-bypass-hook-trust", default_value_t = false)]
    pub bypass_hook_trust: bool,

    /// Tell the agent to use the specified directory as its working root.
    #[clap(long = "cd", short = 'C', value_name = "DIR")]
    pub cwd: Option<PathBuf>,

    /// Additional directories that should be writable alongside the primary workspace.
    #[arg(long = "add-dir", value_name = "DIR", value_hint = clap::ValueHint::DirPath)]
    pub add_dir: Vec<PathBuf>,
}

impl SharedCliOptions {
    pub fn take_config_overrides(&mut self, overrides: &mut CliConfigOverrides) {
        let zai = self.zai;
        let openrouter = self.openrouter;
        if zai {
            self.model = Some("glm-5.3".to_string());
            add_external_provider_overrides(
                overrides,
                "zai",
                "Z.AI",
                "https://api.z.ai/api/v1",
                "ZAI_API_KEY",
            );
            overrides
                .raw_overrides
                .push(r#"model_reasoning_effort="max""#.to_string());
            self.zai = false;
        } else if openrouter {
            self.model = Some("deepseek/deepseek-v4.1-flash".to_string());
            add_external_provider_overrides(
                overrides,
                "openrouter",
                "OpenRouter",
                "https://openrouter.ai/api/v1",
                "OPENROUTER_API_KEY",
            );
            self.openrouter = false;
        }
        if self.astra_ultra
            || self.astra_ultra_low
            || self.astra_ultra_miser
            || self.astra_boss
            || self.astra_boss_low
            || self.astra_boss_miser
            || self.boss.is_some()
        {
            if !zai && !openrouter && self.model.is_none() {
                self.model = Some("gpt-6-astra".to_string());
            }
            if self.astra_ultra_miser || self.astra_boss_miser {
                self.ultra_effort = Some(ReasoningEffort::Low);
            } else if zai {
                self.ultra_effort.get_or_insert(ReasoningEffort::Max);
            } else if openrouter {
                self.ultra_effort.get_or_insert(ReasoningEffort::High);
            } else {
                self.ultra_effort.get_or_insert(ReasoningEffort::Medium);
            }
            overrides
                .raw_overrides
                .push(r#"model_reasoning_effort="ultra""#.to_string());
            for (setting, enabled) in [
                ("astra_boss", self.astra_boss),
                (
                    "astra_ultra_low",
                    self.astra_ultra_low || self.astra_ultra_miser,
                ),
                (
                    "astra_boss_low",
                    self.astra_boss_low || self.astra_boss_miser,
                ),
            ] {
                overrides
                    .raw_overrides
                    .push(format!("features.multi_agent_v2.{setting}={enabled}"));
            }
            overrides
                .raw_overrides
                .push("features.multi_agent_v2.enabled=true".to_string());
            self.astra_ultra = false;
            self.astra_ultra_low = false;
            self.astra_ultra_miser = false;
            self.astra_boss = false;
            self.astra_boss_low = false;
            self.astra_boss_miser = false;
        }
        if let Some(model) = self.boss.take() {
            #[expect(
                clippy::expect_used,
                reason = "Clap validates worker selectors before use"
            )]
            let selected = WorkerModel::parse(&model).expect("worker selector validated by clap");
            let model = toml::Value::String(selected.qualified_model());
            let effort = selected
                .effort
                .or(self.worker_effort.take())
                .unwrap_or(ReasoningEffort::Medium);
            overrides.raw_overrides.push(format!(
                "features.multi_agent_v2.boss_mode={{enabled=true,model={model},reasoning_effort=\"{effort}\"}}"
            ));
        }
        if self.ultra_effort.is_some()
            && !overrides.raw_overrides.iter().any(|override_| {
                override_
                    .split_once('=')
                    .is_some_and(|(key, _)| key.trim() == "model_reasoning_effort")
            })
        {
            overrides
                .raw_overrides
                .push(r#"model_reasoning_effort="ultra""#.to_string());
        }
        if let Some(effort) = self.effort.take() {
            if self.ultra_effort.is_some() {
                self.ultra_effort = Some(effort);
            } else {
                overrides
                    .raw_overrides
                    .push(format!("model_reasoning_effort=\"{effort}\""));
            }
        }
        if self.auto_review {
            overrides
                .raw_overrides
                .push(r#"approvals_reviewer="auto_review""#.to_string());
            overrides
                .raw_overrides
                .push(r#"approval_policy="on-request""#.to_string());
            overrides
                .raw_overrides
                .push(r#"sandbox_mode="workspace-write""#.to_string());
            self.auto_review = false;
        }
        if self.max_context {
            overrides
                .raw_overrides
                .push("use_max_context_window=true".to_string());
            self.max_context = false;
        }
        if self.compact_with_lower {
            overrides
                .raw_overrides
                .push("compact_with_lower=true".to_string());
            self.compact_with_lower = false;
        }
    }

    pub fn inherit_exec_root_options(&mut self, root: &Self) {
        let self_selected_model = self.model.is_some();
        let self_selected_sandbox_mode = self.sandbox_mode.is_some()
            || self.auto_review
            || self.dangerously_bypass_approvals_and_sandbox;
        let Self {
            images,
            model,
            effort,
            astra_ultra,
            astra_ultra_low,
            astra_ultra_miser,
            astra_boss,
            astra_boss_low,
            astra_boss_miser,
            boss,
            worker_effort,
            ultra_effort,
            max_context,
            compact_with_lower,
            zai,
            openrouter,
            oss,
            oss_provider,
            config_profile_v2,
            sandbox_mode,
            auto_review,
            dangerously_bypass_approvals_and_sandbox,
            bypass_hook_trust,
            cwd,
            add_dir,
        } = self;
        let Self {
            images: root_images,
            model: root_model,
            effort: root_effort,
            astra_ultra: root_astra_ultra,
            astra_ultra_low: root_astra_ultra_low,
            astra_ultra_miser: root_astra_ultra_miser,
            astra_boss: root_astra_boss,
            astra_boss_low: root_astra_boss_low,
            astra_boss_miser: root_astra_boss_miser,
            boss: root_boss,
            worker_effort: root_worker_effort,
            ultra_effort: root_ultra_effort,
            max_context: root_max_context,
            compact_with_lower: root_compact_with_lower,
            zai: root_zai,
            openrouter: root_openrouter,
            oss: root_oss,
            oss_provider: root_oss_provider,
            config_profile_v2: root_config_profile_v2,
            sandbox_mode: root_sandbox_mode,
            auto_review: root_auto_review,
            dangerously_bypass_approvals_and_sandbox: root_dangerously_bypass_approvals_and_sandbox,
            bypass_hook_trust: root_bypass_hook_trust,
            cwd: root_cwd,
            add_dir: root_add_dir,
        } = root;

        if boss.is_none()
            && (!*zai && !*openrouter
                || root_boss
                    .as_deref()
                    .and_then(|value| WorkerModel::parse(value).ok())
                    .is_some_and(|worker| worker.provider.is_some()))
        {
            boss.clone_from(root_boss);
        }
        if worker_effort.is_none() {
            worker_effort.clone_from(root_worker_effort);
        }
        if effort.is_none() {
            effort.clone_from(root_effort);
        }
        if model.is_none() {
            model.clone_from(root_model);
        }
        if !self_selected_model
            && !*astra_ultra
            && !*astra_ultra_low
            && !*astra_ultra_miser
            && !*astra_boss
            && !*astra_boss_low
            && !*astra_boss_miser
            && !*zai
            && !*openrouter
        {
            *astra_ultra = *root_astra_ultra;
            *astra_ultra_low = *root_astra_ultra_low;
            *astra_ultra_miser = *root_astra_ultra_miser;
            *astra_boss = *root_astra_boss;
            *astra_boss_low = *root_astra_boss_low;
            *astra_boss_miser = *root_astra_boss_miser;
            *zai = *root_zai;
            *openrouter = *root_openrouter;
        }
        if ultra_effort.is_none() {
            ultra_effort.clone_from(root_ultra_effort);
        }
        if *root_max_context {
            *max_context = true;
        }
        if *root_compact_with_lower {
            *compact_with_lower = true;
        }
        if *root_oss {
            *oss = true;
        }
        if oss_provider.is_none() {
            oss_provider.clone_from(root_oss_provider);
        }
        if config_profile_v2.is_none() {
            config_profile_v2.clone_from(root_config_profile_v2);
        }
        if !self_selected_sandbox_mode {
            *sandbox_mode = *root_sandbox_mode;
            *auto_review = *root_auto_review;
            *dangerously_bypass_approvals_and_sandbox =
                *root_dangerously_bypass_approvals_and_sandbox;
        }
        if !*bypass_hook_trust {
            *bypass_hook_trust = *root_bypass_hook_trust;
        }
        if cwd.is_none() {
            cwd.clone_from(root_cwd);
        }
        if !root_images.is_empty() {
            let mut merged_images = root_images.clone();
            merged_images.append(images);
            *images = merged_images;
        }
        if !root_add_dir.is_empty() {
            let mut merged_add_dir = root_add_dir.clone();
            merged_add_dir.append(add_dir);
            *add_dir = merged_add_dir;
        }
    }

    pub fn apply_subcommand_overrides(&mut self, subcommand: Self) {
        let subcommand_selected_sandbox_mode = subcommand.sandbox_mode.is_some()
            || subcommand.auto_review
            || subcommand.dangerously_bypass_approvals_and_sandbox;
        let Self {
            images,
            model,
            effort,
            astra_ultra,
            astra_ultra_low,
            astra_ultra_miser,
            astra_boss,
            astra_boss_low,
            astra_boss_miser,
            boss,
            worker_effort,
            ultra_effort,
            max_context,
            compact_with_lower,
            zai,
            openrouter,
            oss,
            oss_provider,
            config_profile_v2,
            sandbox_mode,
            auto_review,
            dangerously_bypass_approvals_and_sandbox,
            bypass_hook_trust,
            cwd,
            add_dir,
        } = subcommand;

        if let Some(model) = model {
            self.model = Some(model);
            self.astra_ultra = false;
            self.astra_ultra_low = false;
            self.astra_ultra_miser = false;
            self.astra_boss = false;
            self.astra_boss_low = false;
            self.astra_boss_miser = false;
            self.zai = false;
            self.openrouter = false;
        }
        if astra_ultra
            || astra_ultra_low
            || astra_ultra_miser
            || astra_boss
            || astra_boss_low
            || astra_boss_miser
        {
            self.astra_ultra = astra_ultra;
            self.astra_ultra_low = astra_ultra_low;
            self.astra_ultra_miser = astra_ultra_miser;
            self.astra_boss = astra_boss;
            self.astra_boss_low = astra_boss_low;
            self.astra_boss_miser = astra_boss_miser;
            self.zai = false;
            self.openrouter = false;
        }
        if zai || openrouter {
            if self
                .boss
                .as_deref()
                .and_then(|value| WorkerModel::parse(value).ok())
                .is_none_or(|worker| worker.provider.is_none())
            {
                self.boss = None;
                self.worker_effort = None;
            }
            self.model = None;
            self.astra_ultra = astra_ultra;
            self.astra_ultra_low = false;
            self.astra_ultra_miser = false;
            self.astra_boss = false;
            self.astra_boss_low = false;
            self.astra_boss_miser = false;
            self.zai = zai;
            self.openrouter = openrouter;
        }
        if boss.is_some() {
            self.boss = boss;
        }
        if effort.is_some() {
            self.effort = effort;
        }
        if worker_effort.is_some() {
            self.worker_effort = worker_effort;
        }
        if let Some(ultra_effort) = ultra_effort {
            self.ultra_effort = Some(ultra_effort);
        }
        if max_context {
            self.max_context = true;
        }
        if compact_with_lower {
            self.compact_with_lower = true;
        }
        if oss {
            self.oss = true;
        }
        if let Some(oss_provider) = oss_provider {
            self.oss_provider = Some(oss_provider);
        }
        if let Some(config_profile_v2) = config_profile_v2 {
            self.config_profile_v2 = Some(config_profile_v2);
        }
        if subcommand_selected_sandbox_mode {
            self.sandbox_mode = sandbox_mode;
            self.auto_review = auto_review;
            self.dangerously_bypass_approvals_and_sandbox =
                dangerously_bypass_approvals_and_sandbox;
        }
        if bypass_hook_trust {
            self.bypass_hook_trust = true;
        }
        if let Some(cwd) = cwd {
            self.cwd = Some(cwd);
        }
        if !images.is_empty() {
            self.images = images;
        }
        if !add_dir.is_empty() {
            self.add_dir.extend(add_dir);
        }
    }
}

fn add_external_provider_overrides(
    overrides: &mut CliConfigOverrides,
    provider_id: &str,
    provider_name: &str,
    base_url: &str,
    env_key: &str,
) {
    let command = std::env::current_exe()
        .ok()
        .and_then(|path| path.into_os_string().into_string().ok())
        .unwrap_or_else(|| "cowdex".to_string());
    let command = toml::Value::String(command);
    let provider_name = toml::Value::String(provider_name.to_string());
    let base_url = toml::Value::String(base_url.to_string());
    let credential_args = toml::Value::Array(
        ["internal-provider-credential", provider_id, env_key]
            .map(|value| toml::Value::String(value.to_string()))
            .to_vec(),
    );

    overrides
        .raw_overrides
        .push(format!(r#"model_provider="{provider_id}""#));
    overrides
        .raw_overrides
        .push("features.standalone_web_search=true".to_string());
    overrides.raw_overrides.push(format!(
        "model_providers.{provider_id}={{ name={provider_name}, base_url={base_url}, wire_api=\"responses\", auth={{ command={command}, args={credential_args} }} }}"
    ));
}

#[cfg(test)]
#[path = "shared_options_tests.rs"]
mod tests;
