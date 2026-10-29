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

**Background jobs.** Every model `bash` is a live job you can watch in `/bash`. Long-running work can start as true background (`run_in_background`), or you can press **Ctrl+B** mid-flight to detach a foreground bash (or blocking sub-agent) without killing it — the turn unblocks; koma nudges the agent when the job finishes.

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

Headless one-shot (session daemon, not standalone):

```sh
koma run --prompt 'ping' --once --timeout 120
koma run --prompt-file ./instructions.txt --name batch-1 --workdir "$PWD"
```

The [Koma Open SDKs](src-core-sdk/README.md) (Python and JavaScript/TypeScript) can drive sessions
over `koma sdk serve`. The SDKs use an installed Koma binary; they do not download or
build the engine. See the SDK guide for local/artifact installation and compatibility checks.

Unknown commands (e.g. `koma docker`) print help and exit 1.

Works on Linux, macOS, and Windows. On Windows, use the PowerShell installer — it downloads and runs the MSI directly. A separate Git Bash install is not required: the MSI uses Git for Windows when it is already present, and otherwise ships a fallback shell:

```powershell
irm https://koma.run/install.ps1 | iex
```

The MSI is the canonical Windows package. Security tools and full-internet/research mode are not supported on Windows.

### Offline terminal and source builds

Official binaries include Bun 1.4.3, OpenTUI's native library, parser worker,
web-tree-sitter, and all shipped grammar/query assets. First launch extracts and
verifies them under Koma's user data directory (`~/.koma/term` on Unix,
`%LOCALAPPDATA%\koma\term` on Windows). No Bun installation, package manager,
checkout, or internet connection is needed to open the terminal. Provider requests
still need the provider's connection. Missing or corrupt cached files are restored
from the executable; new payload versions use separate cache directories.

The OpenTUI terminal requires macOS 13+, Linux glibc 2.35+ for official Koma
binaries, or Windows 10 version 1809+. See [Bun's platform requirements](https://bun.sh/docs/installation).
Existing GUI system-library requirements remain. `koma legacy` stays available;
the Rust macOS deployment setting remains 11.0 for other entrypoints.

Source builds require Python 3, Rust, and network access during payload preparation:

```sh
python3 scripts/prepare-terminal.py
cargo build --locked --release -p agent
# Or use ./build.sh, which prepares the payload before building.
```

Preparation downloads checksum-pinned Bun archives and installs production
packages from `src-term/bun.lock` in a fresh staging directory. For cross builds,
pass `--target <Rust target triple>` to preparation and Cargo. Cargo rejects
missing, stale, corrupt, or wrong-target payloads, including headless builds.
Build outputs live under `target/terminal`; retain this directory for offline
rebuilds of unchanged terminal inputs. Python is a build prerequisite only.

For terminal development, install development packages with
`cd src-term && bun install --frozen-lockfile` using Bun 1.4.3, then explicitly set
`KOMA_TERM_DIR` to that directory. The launcher still uses its embedded Bun.
Ordinary launches always use the embedded terminal, even inside a checkout.

### Desktop GUI (optional)

The `koma gui` desktop client (a wry webview hosting xterm.js) is behind the `gui` cargo feature at build time, but you don't need to build it yourself on most platforms:

- **macOS** — the install.sh binary already has `gui` baked in; `koma gui` just works. install.sh also adds a user-level app at `~/Applications/Koma.app` that opens the desktop client from Finder, Launchpad, and Spotlight.
- **Linux** — the raw install.sh binary is GUI-featured and needs WebKitGTK 4.1, GTK 3, and libXtst at runtime. On Debian and Ubuntu, install.sh installs those shared libraries (including the GTK and soup libraries WebKit pulls in; `libgtk-3-0` or `libgtk-3-0t64`, whichever apt has). The `koma` launcher still checks for them and prints an install command if they are missing. install.sh also installs a **user-level** `.desktop` entry + icons under `~/.local/share` so Koma appears in the app drawer (useful on immutable/atomic desktops like Fedora Bluefin where `.deb` is awkward). If you prefer a package that handles dependencies automatically, grab the `.deb` or `.AppImage` instead; both are `gui`-featured and add a system desktop entry that launches straight into `koma gui`. Prebuilt Linux binaries target **glibc 2.35 (Ubuntu 22.04 LTS)** and run on 22.04 and all newer releases.
- **Windows** — `irm https://koma.run/install.ps1 | iex` installs the `.msi` (GUI build with WebView2 runtime bundled, plus a fallback shell so agent commands and the built-in terminal are not stuck on `cmd`). WebView2 ships with Windows 11 and most patched Windows 10 machines already. If Git for Windows is installed, koma uses that Bash instead of the bundled one. Security tools and full-internet/research mode are not supported on Windows.

Grab the platform installer from the [latest release](https://github.com/aula-id/koma/releases/latest): `koma-x64.deb` / `koma-arm64.deb`, `koma-x64.AppImage` / `koma-arm64.AppImage`, `koma-x64.msi`.

Building the GUI from source yourself (e.g. a gui-featured Linux binary):

```sh
python3 scripts/prepare-terminal.py
cargo build --locked -p agent --features gui
```

For Linux GUI flicker or black frames on Hyprland/Wayland with Intel Ivy Bridge graphics, see [Linux graphics troubleshooting](docs/LINUX_GRAPHICS.md).

Linux build prerequisite: `webkit2gtk-4.1` and its dev headers (e.g. `libwebkit2gtk-4.1-dev` on Debian/Ubuntu), plus GTK3 and libsoup3. macOS and Windows use the system-provided webview (WebKit / WebView2) — no extra deps. The default build enables the GUI; use `--no-default-features` for a terminal-only build. The complete terminal payload is included in both.

More at **[koma.run](https://koma.run)**.

---

*Curious how it works under the hood? See [`ARCHITECTURE.md`](docs/ARCHITECTURE.md).*  

*Extending the desktop GUI? See [`WEBGUI_SIDEBAR_TABS.md`](docs/WEBGUI_SIDEBAR_TABS.md).*  

*Adding a new GUI feature? See [`ADDING_GUI_FEATURES.md`](docs/ADDING_GUI_FEATURES.md).*  
*Connecting GUI tabs to Rust? See [`WEBGUI_IPC.md`](docs/WEBGUI_IPC.md).*
