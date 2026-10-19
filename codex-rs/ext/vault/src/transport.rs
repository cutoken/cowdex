use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Profile {
    pub socket: PathBuf,
    pub ticket: String,
    #[serde(default)]
    pub scope: Vec<String>,
}

pub(super) async fn read_profile(path: PathBuf) -> Result<Profile, String> {
    #[cfg(unix)]
    {
        use std::io::Read;
        use std::os::unix::fs::OpenOptionsExt;
        tokio::task::spawn_blocking(move || {
            let path = if path.as_os_str().is_empty() {
                std::env::var_os("COWDEX_VAULT_HOME")
                    .map(PathBuf::from)
                    .or_else(|| {
                        std::env::var_os("HOME")
                            .map(|home| PathBuf::from(home).join(".local/share/cowdex-vault"))
                    })
                    .ok_or("Cannot locate the default vault")?
                    .join("grant.json")
            } else {
                path
            };
            if !path.is_absolute() {
                return Err("Vault attachment profile must use an absolute path".into());
            }
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path)
                .map_err(
                    |_| "Open the vault page, unlock it, and make credentials available to Cowdex",
                )?;
            if !file
                .metadata()
                .map_err(|_| "Cannot inspect vault attachment profile")?
                .is_file()
            {
                return Err("Vault attachment profile must be a regular file".into());
            }
            let mut bytes = Vec::new();
            file.take(8193)
                .read_to_end(&mut bytes)
                .map_err(|_| "Cannot read vault attachment profile")?;
            if bytes.len() > 8192 {
                return Err("Vault attachment profile is too large".into());
            }
            let profile: Profile =
                serde_json::from_slice(&bytes).map_err(|_| "Invalid vault attachment profile")?;
            if !profile.socket.is_absolute()
                || profile.ticket.is_empty()
                || profile.ticket.len() > 4096
            {
                return Err("Invalid vault attachment profile".into());
            }
            Ok(profile)
        })
        .await
        .map_err(|_| "Cannot read vault attachment profile")?
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err("Vault attachment currently requires Unix sockets".into())
    }
}

pub(super) async fn exchange(socket: &Path, value: Value) -> Result<Value, String> {
    #[cfg(unix)]
    {
        use tokio::io::AsyncBufReadExt;
        use tokio::io::AsyncReadExt;
        use tokio::io::AsyncWriteExt;
        let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let mut stream = tokio::net::UnixStream::connect(socket)
                .await
                .map_err(|_| "Vault broker is unavailable")?;
            let mut request = serde_json::to_vec(&value).map_err(|_| "Invalid vault request")?;
            request.push(b'\n');
            if request.len() > 65536 {
                return Err("Vault request is too large".into());
            }
            stream
                .write_all(&request)
                .await
                .map_err(|_| "Vault broker request failed")?;
            let mut reader = tokio::io::BufReader::new(stream.take(65537));
            let mut bytes = Vec::new();
            reader
                .read_until(b'\n', &mut bytes)
                .await
                .map_err(|_| "Vault broker response failed")?;
            if bytes.len() > 65536 || bytes.last() != Some(&b'\n') {
                return Err("Invalid vault broker response".into());
            }
            let value: Value =
                serde_json::from_slice(&bytes).map_err(|_| "Invalid vault broker response")?;
            if value.get("error").is_some() {
                return Err(
                    "Vault broker rejected the request; check grant scope and expiry".into(),
                );
            }
            Ok(value)
        })
        .await;
        result.map_err(|_| "Vault broker timed out")?
    }
    #[cfg(not(unix))]
    {
        let _ = (socket, value);
        Err("Vault attachment currently requires Unix sockets".into())
    }
}
