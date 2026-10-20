# Cowdex Vault

Run `cowdex-vault` to open the local browser manager. The executable includes its UI and needs no Python, pip, or other runtime. Release packages support Linux, macOS, and Windows on x64 and ARM64.

## Use

1. Create or unlock the vault with a password.
2. Add a provider credential and choose its allowed methods and path prefix.
3. In Cowdex, run `/vault attach` or `/vault attach --scope alias`.
4. Approve the matching pairing code in the browser, then repeat the attach command.

Access lasts up to 24 hours. `/vault detach` revokes a session family's access; **Lock vault** revokes all access and closes the manager. Run `cowdex-vault` again to reopen it. Blank token fields preserve existing values when editing a credential. Changing its policy revokes affected attachments.

The vault keeps API keys in the broker. Agents use `vault_request` with an alias, method, relative API path, and optional body. Parent sessions and workers share an attachment; resumed sessions start detached.

## Storage

The default directory is `~/.local/share/cowdex-vault` on every platform. Set `COWDEX_VAULT_HOME` to an absolute path for another directory, using the same setting for Cowdex and the manager. Existing Python vault files open with their original password; no conversion is required.

Credentials remain encrypted with scrypt and AES-256-GCM. Local attachment profiles hold temporary pairing authority. Unix systems use a private socket; Windows uses loopback TCP with the same approval and session protocol. A same-user broker does not isolate secrets from processes that can inspect its memory.

## Build

```sh
go build -trimpath -ldflags="-s -w" -o cowdex-vault .
go test ./...
```

Go 1.26+ is required to build, not to run. `--serve` runs in the foreground, `--no-browser` prints the URL, `--port` selects the browser port, and `--ttl` sets the unlock duration in seconds (maximum 86400).
