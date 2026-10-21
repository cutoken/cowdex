# 🐮 Cowdex

Cowdex is a Codex fork focused on saving tokens and a slower, deliberate release cycle to reduce exposure to upstream regressions. It adds worker waits and completion events instead of constant status checking, Boss mode for choosing worker models and effort, Ultra with all model-supported effort levels (low through max), guardrails against overengineering and redundant work, and DeepSeek via OpenRouter plus Z.ai Coding Plan support.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/cutoken/cowdex/main/scripts/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
cowdex login
cowdex
```

Packages include matching support binaries for Linux, macOS and Windows, on x64 and ARM64. Installation and updates use the platform shell, with no Python, Node.js, or Go runtime required. On Windows, run:

```powershell
irm https://raw.githubusercontent.com/cutoken/cowdex/main/scripts/install.ps1 | iex
```

Run the installer again to update.

```sh
cowdex --ultra
cowdex --ultra --model gpt-6.1-sol --boss sol --worker-effort medium
cowdex resume --zai
```

Live token counters show total tokens (`T`), cached and uncached input (`CI`/`NI`), reasoning output (`OUT_R`), and total output (`OUT_T`).

Lower logging overhead and bounded history caches keep storage lean. `/cleanup` shrinks session files while preserving resume support, backs up originals, removes stale subagents, and protects running sessions.

Use `cowdex keepwarm` to keep additional accounts warm. It signs in separately and runs a research task without replacing your current login.

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
