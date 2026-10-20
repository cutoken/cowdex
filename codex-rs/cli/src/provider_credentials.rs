use clap::Parser;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Parser)]
pub(crate) struct Command {
    provider: String,
    env_key: String,
}

#[derive(Default, Deserialize, Serialize)]
struct Credentials {
    #[serde(default)]
    providers: HashMap<String, ProviderCredential>,
}

#[derive(Deserialize, Serialize)]
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

    let credentials_path = credentials_path()?;
    let contents = std::fs::read_to_string(&credentials_path).map_err(|err| {
        anyhow::anyhow!(
            "No {} available; set it or run cowdex login --provider {} --with-api-key ({}: {err})",
            command.env_key,
            command.provider,
            credentials_path.display()
        )
    })?;
    let credentials: Credentials = serde_json::from_str(&contents).map_err(|err| {
        anyhow::anyhow!(
            "failed to parse Cowdex credentials at {}: {err}",
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
                "No saved key for {}; set {} or run cowdex login --provider {} --with-api-key",
                command.provider,
                command.env_key,
                command.provider
            )
        })?;
    println!("{api_key}");
    Ok(())
}

fn credentials_path() -> anyhow::Result<PathBuf> {
    Ok(codex_core::config::find_codex_home()?
        .as_path()
        .join("provider-keys.json"))
}

pub(crate) fn save(provider: &str, api_key: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!api_key.trim().is_empty(), "API key cannot be empty");
    let path = credentials_path()?;
    let parent = path.parent().expect("credential file has a parent");
    std::fs::create_dir_all(parent)?;
    let mut credentials: Credentials = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Credentials::default(),
        Err(error) => return Err(error.into()),
    };
    credentials.providers.insert(
        provider.to_string(),
        ProviderCredential {
            api_key: api_key.trim().to_string(),
        },
    );
    let mut output = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        output
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    serde_json::to_writer(&mut output, &credentials)?;
    output.as_file().sync_all()?;
    output.persist(path)?;
    eprintln!("Saved {provider} API key for Cowdex.");
    Ok(())
}
