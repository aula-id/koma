# koma

**Your terminal, with a brain.**

koma is a fast, native AI coding agent that lives in your terminal — reading your code, shipping features, and running your tools without you ever leaving the command line.

→ **[koma.run](https://koma.run)**

---

## Why koma

- **Native and fast.** Written in Rust. No Electron, no browser tab, no lag — a crisp TUI that starts instantly and stays out of your way.
- **It actually does the work.** koma reads your code, edits files, runs commands, and verifies its own changes. You orchestrate; it executes.
- **Bring your own models.** Wire up your providers and assign different models to different roles — planning, coding, review — and switch on the fly.
- **Yours to control.** Every tool call runs behind an approval gate you set. Nothing touches your machine without your say-so.

## What's inside

**Parallel sub-agents.** Hand a chunk of work to agents that run side by side, then fold their results back in. Big refactors, broad audits, multi-file sweeps — fanned out, not serialized.

**Background jobs.** Fire off long-running commands and keep working. koma watches them, lets the agent grep and tail their output, and nudges it the moment they finish.

**Multi-session, detachable.** Run many sessions at once, each in its own tab. Detach the daemon, close the laptop, come back later — your work is exactly where you left it.

**Internet access.** Search and fetch from the web inline, or flip to Full mode for real browser-powered scraping when a page fights back.

**Security toolkit.** A curated, opt-in suite of security tools wired straight into the agent for authorized testing and research.

**MCP-ready.** Connect any Model Context Protocol server and its tools show up for the agent automatically.

**Memory that sticks.** A per-project memory carries conventions, decisions, and context across sessions — so koma stops relearning your codebase every morning.

**Vision.** Paste a screenshot. koma sees it.

**Cost in plain sight.** A live usage dashboard shows exactly what every turn costs, backed by a full ledger you can audit.

**Self-updating.** Run `koma update` and you're on the latest in seconds.

## Get koma

```sh
curl -fsSL https://koma.run/install.sh | sh
```

Installs to `~/.local/bin` — no sudo required. Then run `koma` and start a session.

Works on Linux, macOS, and Windows. On Windows, run that same command from a Git Bash terminal — it installs `koma.exe`. Windows needs [Git for Windows](https://git-scm.com/download/win) (koma's shell tool runs commands through Git Bash).

### Desktop GUI (optional)

The `koma gui` desktop client (a wry webview hosting xterm.js) is behind the `gui` cargo feature at build time, but you don't need to build it yourself on most platforms:

- **macOS** — the install.sh binary already has `gui` baked in; `koma gui` just works.
- **Linux** — the raw install.sh binary is GUI-featured but requires `libwebkit2gtk-4.1-0` and `libgtk-3-0` at runtime. On Linux, install.sh ships a launcher (`koma`) that checks for these libraries and prints install commands if they are missing. If you prefer a package that handles dependencies automatically, grab the `.deb` or `.AppImage` instead; both are `gui`-featured and add a desktop entry that launches straight into `koma gui`. Prebuilt Linux binaries target **glibc 2.35 (Ubuntu 22.04 LTS)** and run on 22.04 and all newer releases.
- **Windows** — the `.msi` installer is the GUI build (it installs the WebView2 runtime for you if it's missing; WebView2 ships with Windows 11 and most patched Windows 10 machines already). The raw `koma-windows-x64.exe` from install.sh is TUI-only.

Grab the platform installer from the [latest release](https://github.com/aula-id/koma/releases/latest): `koma-x64.deb` / `koma-arm64.deb`, `koma-x64.AppImage` / `koma-arm64.AppImage`, `koma-x64.msi`.

Building the GUI from source yourself (e.g. a gui-featured Linux binary):

```sh
cargo build -p agent --features gui
```

Linux build prerequisite: `webkit2gtk-4.1` and its dev headers (e.g. `libwebkit2gtk-4.1-dev` on Debian/Ubuntu), plus GTK3 and libsoup3. macOS and Windows use the system-provided webview (WebKit / WebView2) — no extra deps. The default `cargo install` / build pulls none of these — the GUI deps (`wry`, `tao`, `portable-pty`) are optional and only compiled with `--features gui`.

More at **[koma.run](https://koma.run)**.

---

*Curious how it works under the hood? See [`ARCHITECTURE.md`](docs/ARCHITECTURE.md).*  

*Extending the desktop GUI? See [`WEBGUI_SIDEBAR_TABS.md`](docs/WEBGUI_SIDEBAR_TABS.md).*  

*Adding a new GUI feature? See [`ADDING_GUI_FEATURES.md`](docs/ADDING_GUI_FEATURES.md).*  
*Connecting GUI tabs to Rust? See [`WEBGUI_IPC.md`](docs/WEBGUI_IPC.md).*
