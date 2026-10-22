//! Resolves fixed worker models without changing the lead agent's provider.

use crate::config::Config;
use codex_config::ConfigLayerEntry;
use codex_config::ConfigLayerSource;
use codex_config::ConfigLayerStack;
use codex_features::Feature;
use codex_model_provider_info::ModelProviderInfo;
use codex_protocol::config_types::ModelProviderAuthInfo;
use codex_protocol::protocol::BossModeSettings;
use codex_protocol::worker_model::WorkerModel;
use std::num::NonZeroU64;

pub(crate) fn ensure_provider(config: &mut Config, provider: &str) -> Result<(), String> {
    if config
        .config_layer_stack
        .effective_config()
        .get("model_providers")
        .and_then(|providers| providers.get(provider))
        .is_some()
    {
        return Ok(());
    }
    if !config.model_providers.contains_key(provider) {
        let (name, base_url, env_key) = match provider {
            "zai" => ("Z.AI", "https://api.z.ai/api/v1", "ZAI_API_KEY"),
            "openrouter" => (
                "OpenRouter",
                "https://openrouter.ai/api/v1",
                "OPENROUTER_API_KEY",
            ),
            _ => return Err(format!("Model provider `{provider}` not found")),
        };
        let executable = config
            .codex_self_exe
            .clone()
            .or_else(|| std::env::current_exe().ok())
            .ok_or_else(|| "Could not locate Cowdex for worker provider credentials".to_string())?;
        #[expect(clippy::expect_used, reason = "The auth timeout is a nonzero constant")]
        let timeout_ms = NonZeroU64::new(/*n*/ 5_000).expect("nonzero auth timeout");
        config.model_providers.insert(
            provider.to_string(),
            ModelProviderInfo {
                name: name.to_string(),
                base_url: Some(base_url.to_string()),
                auth: Some(ModelProviderAuthInfo {
                    command: executable.to_string_lossy().into_owned(),
                    args: ["internal-provider-credential", provider, env_key]
                        .map(str::to_string)
                        .to_vec(),
                    timeout_ms,
                    refresh_interval_ms: 300_000,
                    cwd: config.cwd.clone(),
                }),
                ..Default::default()
            },
        );
    }
    // Role application reloads config from layers, so the worker's provider must
    // be present there as well as in the resolved runtime map.
    let mut layers = config
        .config_layer_stack
        .all_layers_low_to_high()
        .cloned()
        .collect::<Vec<_>>();
    let definition = toml::Value::try_from(&config.model_providers[provider])
        .map_err(|error| error.to_string())?;
    let definitions =
        toml::Value::Table([(provider.to_string(), definition)].into_iter().collect());
    let layer = toml::Value::Table(
        [("model_providers".to_string(), definitions)]
            .into_iter()
            .collect(),
    );
    layers.push(ConfigLayerEntry::new(
        ConfigLayerSource::SessionFlags,
        layer,
    ));
    config.config_layer_stack = ConfigLayerStack::new(
        layers,
        config.config_layer_stack.requirements().clone(),
        config.config_layer_stack.requirements_toml().clone(),
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn apply(config: &mut Config, boss: &BossModeSettings) -> Result<(), String> {
    let selected = WorkerModel::parse(&boss.model)?;
    let provider = selected.provider.or_else(|| {
        matches!(config.model_provider_id.as_str(), "zai" | "openrouter").then_some("openai")
    });
    if let Some(provider) = provider {
        ensure_provider(config, provider)?;
        config.model_provider = config.model_providers[provider].clone();
        config.model_provider_id = provider.to_string();
        config.service_tier = None;
    }
    if selected.provider.is_some() {
        config
            .features
            .enable(Feature::StandaloneWebSearch)
            .map_err(|error| error.to_string())?;
    }
    config.model = Some(selected.model.to_string());
    config.model_reasoning_effort = Some(boss.reasoning_effort.clone());
    Ok(())
}
