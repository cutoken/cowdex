# Cowdex

Cowdex is a Codex fork with agent teams, provider switching, and controls for long-running sessions.

## Install

```sh
curl -fsSL https://github.com/cutoken/cowdex/releases/latest/download/install.py | python3 - --repo cutoken/cowdex
export PATH="$HOME/.local/bin:$PATH"
cowdex login
cowdex
```

Packages include matching support binaries for Linux, macOS and Windows, on x64 and ARM64. The installer needs Python 3.10+; Cowdex and its vault do not. On Windows, download a release archive and run `py install.py`.

## Features

- **Agent teams:** Ultra enables delegation. Boss fixes worker models and effort; `/boss` changes those defaults during a session.
- **Providers:** OpenAI, Z.ai Coding Plan, and OpenRouter. Sessions can resume across providers. Z.ai and OpenRouter workers use the lead's provider and model.
- **Long sessions:** prompt-prefix caching, idle worker unloading, compaction with message ages, timers, retry recovery, and account changes without restarting.
- **Controls:** `/model`, `/limit_agents`, `/goal`, `/cleanup`, and `/vault`. Cleanup protects running sessions and backs up trimmed history.
- **Vault:** a standalone browser manager for encrypted credentials, scoped access, approvals, and revocation. Run `cowdex-vault`.

```sh
cowdex --ultra
cowdex --ultra --model gpt-6.1-sol --boss sol --worker-effort medium
cowdex resume --zai
```

## Provider keys

Use `ZAI_API_KEY` or `OPENROUTER_API_KEY`, or save a key through stdin:

```sh
printenv ZAI_API_KEY | cowdex login --provider zai --with-api-key
printenv OPENROUTER_API_KEY | cowdex login --provider openrouter --with-api-key
```

Saved keys use `$CODEX_HOME/provider-keys.json`. Settings and sessions default to `~/.codex`; set `CODEX_HOME` for separate state. Cowdex installs separately from regular Codex and preserves running installations when updating.

## Build

Use Rust 1.95.0, Go 1.27.3, and Python 3.11+:

```sh
python3 scripts/build_codex_package.py --variant cowdex --cargo-profile release \
  --package-dir dist/package --archive-output dist/cowdex-package.tar.gz
```

See the [release workflow](.github/workflows/cowdex-release.yml) for platform setup. macOS packages are not signed or notarized.
