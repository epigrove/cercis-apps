# cercis-apps (apps)

**English** | [中文](README.zh-CN.md)

The Cercis app: the user interface on every platform, and the **plugin host** (it discovers
plugins, grants permissions and calls them as subprocesses).

```
apps/
└── desktop/                desktop (macOS first; Linux / Windows following)
    ├── crates/cercis-app/  one executable: local HTTP server + built-in web UI
    │   ├── src/{main,http,api,plugin_host,market_host,icons,bundled}.rs
    │   └── src/web/index.html   the UI (compiled into the binary, never read from disk)
    ├── packaging/          per-platform packaging scripts (linux / macos / windows)
    └── ui-tests/           automated UI acceptance (headless Playwright, unattended)
```

Tablets and phones need no separate client today: run `cercis-app --lan` on a computer, and it
prints a token-bearing LAN address you open in the tablet browser and add to the home screen (a
native mobile client is still to come; the assessment lives in `cercis-docs`).

## Building

The app **contains no data logic** — it links the kernel library (`cercis-core`) directly. Source
builds therefore expect the kernel repository **one level up**: the workspace layout is
`<anywhere>/{kernel, kits/basic, apps}`, because `apps/desktop/Cargo.toml` declares
`path = "../../kernel/crates/cercis-core"`.

```bash
# from <anywhere>/apps/desktop
cargo run -- --no-browser --port 8765     # opens http://127.0.0.1:8765
```

Build an installer (kernel and basic kit inside; **unsigned**, so the first launch needs a manual
approval step):

```bash
bash packaging/macos/build.sh      # produces Cercis.app and a distributable zip
bash packaging/linux/build.sh      # installs into ~/.local (with a desktop entry)
```

Automated UI acceptance (it builds a demo project, starts the app and drives a headless browser —
no human involved):

```bash
bash ui-tests/run-ui-acceptance.sh; cat output/playwright/UI-REPORT.md
```

## Plugin calling contract

```text
<plugin entry> <command…> --project <project dir> --json [--cercis <kernel command>] [--allow-write]
```

- **Project plugins must be trusted before they run**; global plugins (in `~/.cercis/plugins`,
  or wherever `CERCIS_GLOBAL_PLUGINS` points) count as installed.
- `--allow-write` is injected only when the manifest declares `write`: the app is the gatekeeper.
- Plugins are **subprocesses**: a crash or a hang cannot damage the UI or the data.
- If a plugin is missing, its buttons disappear and say what is missing; opening, browsing and
  validating keep working.

Environment variables (developer-facing):

| Variable | Purpose |
|----------|---------|
| `CERCIS_GLOBAL_PLUGINS` | the global plugin directory (default `~/.cercis/plugins`; the old `CERCIS_PLUGINS` still works) |
| `CERCIS_BIN` | path to the kernel command line (defaults to PATH or a sibling of the app) |
| `CERCIS_BUNDLED_KITS` | where the bundled basic kit lives (installers ship it; the app installs it into the global plugin directory on first launch) |

## Form factors

| Form factor | Focus |
|-------------|-------|
| Desktop GUI | information density, batch editing, import/export |
| Desktop CLI | automation, AI calls (same rights as the GUI) |
| Tablet | browsing, handwriting, split view, photos |
| Phone | quick notes, scanning, photos, to-dos; offline first |
