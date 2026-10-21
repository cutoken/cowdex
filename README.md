# 🐮 Cowdex

Cowdex is a Codex fork focused on saving tokens and a slower, deliberate release cycle to reduce exposure to upstream regressions.

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

Run `cowdex update` or the installer again to update. Cowdex also checks for new releases and offers an update prompt.

Project management for your agents: [AgentCron](https://agentcron.ai).

```sh
cowdex --ultra
cowdex --worker sol:medium
cowdex --model gpt-6.1-sol --effort high --worker luna:low
cowdex resume --zai
cowdex --openrouter
```

`--worker` enables Boss mode. Omit its value for `sol:medium`, or omit either part: `sol` uses medium effort; `:high` uses Sol. Without `--model` or `--effort`, Boss keeps its existing main-agent defaults: Astra at medium effort. Older switches remain accepted.

## Features

- **Worker events.** Agents wait for worker completion instead of constantly checking status.

  Fewer wasted tokens on polling.

- **Boss mode.** Choose fixed worker models and effort with `--worker sol:medium`.

  Spend less on delegated work while keeping a stronger main agent.

- **Ultra.** Let the main agent choose workers, with every model-supported effort level from low through max.

  Match reasoning cost to the task.

- **Engineering guardrails.** Instructions discourage unnecessary abstractions, redundant tests, and unrelated changes.

  Less scope creep and rework.

- **OpenRouter and Z.ai.** Use either provider for the main agent and its workers; resume conversations across providers.

  Keep working with your preferred provider.

- **Automatic retries.** Recover from model, server, network, and stream failures, including “Selected model is at capacity.” Server errors retry every 60 seconds without a retry limit; closed WebSockets fall back to HTTPS.

  Less babysitting when providers have a bad day.

- **Live token counts.** Track total tokens (`T`), cached and uncached input (`CI`/`NI`), reasoning output (`OUT_R`), and total output (`OUT_T`).

  See where your tokens go.

- **Lighter storage.** Lower logging overhead and bounded history caches. `/cleanup` shrinks session files while preserving resume support, backs up originals, removes stale subagents, and protects running sessions.

  Less SSD churn and runaway disk usage.

- **Keepwarm.** `cowdex keepwarm` signs into additional accounts and runs a research task without replacing your current login.

  Keep accounts warm without disrupting your session.

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
