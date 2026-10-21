# Cowdex

Cowdex is a Codex fork focused on saving tokens and a slower, deliberate release cycle to reduce exposure to upstream regressions. It adds worker waits and completion events instead of constant status checking, Boss mode for choosing worker models and effort, Ultra with all model-supported effort levels (low through max), guardrails against overengineering and redundant work, and DeepSeek via OpenRouter plus Z.ai Coding Plan support.

## Install

```sh
curl -fsSL https://github.com/cutoken/cowdex/releases/latest/download/install.py | python3 - --repo cutoken/cowdex
export PATH="$HOME/.local/bin:$PATH"
cowdex login
cowdex
```

Packages include matching support binaries for Linux, macOS and Windows, on x64 and ARM64. The installer needs Python 3.10+; Cowdex and its vault do not. On Windows, download a release archive and run `py install.py`.

Cowdex checks its own releases at startup and prompts when an update is available. Run `cowdex update` to update manually; updates also need Python.

```sh
cowdex --ultra
cowdex --ultra --model gpt-6.1-sol --boss sol --worker-effort medium
cowdex resume --zai
```

Lower logging overhead and bounded history caches keep storage lean. `/cleanup` shrinks session files while preserving resume support, backs up originals, removes stale subagents, and protects running sessions.

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
