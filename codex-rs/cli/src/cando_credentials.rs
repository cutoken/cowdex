use clap::Parser;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Parser)]
pub(crate) struct Command {
    provider: String,
    env_key: String,
}

#[derive(Deserialize)]
struct Credentials {
    #[serde(default)]
    providers: HashMap<String, ProviderCredential>,
}

#[derive(Deserialize)]
struct ProviderCredential {
    api_key: String,
}

pub(crate) fn run(command: &Command) -> anyhow::Result<()> {
    if let Some(api_key) = std::env::var(&command.env_key)
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        println!("{api_key}");
        return Ok(());
    }

    let credentials_path = if let Some(path) = std::env::var_os("CANDO_CREDENTIALS_PATH") {
        PathBuf::from(path)
    } else {
        let config_dir = std::env::var_os("CANDO_CONFIG_DIR")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|home| home.join(".cando")))
            .ok_or_else(|| anyhow::anyhow!("could not resolve the Cando credential directory"))?;
        config_dir.join("credentials.yaml")
    };
    let contents = std::fs::read_to_string(&credentials_path).map_err(|err| {
        anyhow::anyhow!(
            "failed to read Cando credentials at {}: {err}",
            credentials_path.display()
        )
    })?;
    let credentials: Credentials = serde_yaml::from_str(&contents).map_err(|err| {
        anyhow::anyhow!(
            "failed to parse Cando credentials at {}: {err}",
            credentials_path.display()
        )
    })?;
    let api_key = credentials
        .providers
        .get(&command.provider)
        .map(|provider| provider.api_key.trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Cando credentials do not contain providers.{}.api_key; set {} instead",
                command.provider,
                command.env_key
            )
        })?;
    println!("{api_key}");
    Ok(())
}
