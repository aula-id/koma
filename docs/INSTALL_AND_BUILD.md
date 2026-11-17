# Install and build Koma

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

The [Koma Open SDKs](../src-core-sdk/README.md) (Python and JavaScript/TypeScript) can drive sessions
over `koma sdk serve`. The SDKs use an installed Koma binary; they do not download or
build the engine. See the SDK guide for local/artifact installation and compatibility checks.

Unknown commands (e.g. `koma docker`) print help and exit 1.

Works on Linux, macOS, and Windows (x64 on Windows; Windows 11 on ARM64 runs it as an emulated x64 app). On Windows, use the PowerShell installer. It downloads the MSI, checks it against the release's `SHA256SUMS`, and installs it silently: one Windows permission prompt (none if the window is already an administrator) and no installer windows. A separate Git Bash install is not required: the MSI uses Git for Windows when it is already present, and otherwise ships a fallback shell:

```powershell
irm https://koma.run/install.ps1 | iex
```

The MSI is the canonical Windows package. Double-clicking it still works but shows the installer dialogs, and because it is not code-signed Windows SmartScreen may warn about a hand-downloaded copy. Security tools and full-internet/research mode are not supported on Windows.

Windows installer options (set the variable in the same window first, then run the command):

| Variable | Effect |
| --- | --- |
| `KOMA_VERSION` | Install this release tag instead of the latest, e.g. `v0.2.27` or `0.2.27`. |
| `KOMA_RELEASE_BASE` | Download `koma-x64.msi` and `SHA256SUMS` from this base URL instead of GitHub releases. |
| `KOMA_SKIP_CHECKSUM=1` | Install without the `SHA256SUMS` check. A missing `SHA256SUMS` otherwise stops the install. |
| `KOMA_FORCE_CLOSE=1` | Stop a running koma without asking (for scripts). Otherwise the installer asks first. |
| `KOMA_UNINSTALL=1` | Remove koma quietly instead of installing it. Sessions and settings are kept. |

```powershell
$env:KOMA_VERSION = 'v0.2.27'; irm https://koma.run/install.ps1 | iex
$env:KOMA_UNINSTALL = '1'; irm https://koma.run/install.ps1 | iex
```

The installer adds koma to the PATH of the user who runs the command, refreshes the current window's PATH, and explains any failure in plain English. If Windows Installer fails, the verbose log is copied to `%LOCALAPPDATA%\koma\logs\install-<time>.log` and its path is printed.

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

The `koma gui` desktop client (a wry webview hosting the web UI built from `src-webgui`) is behind the `gui` cargo feature at build time, but you don't need to build it yourself on most platforms:

- **macOS** — the install.sh binary already has `gui` baked in; `koma gui` just works. install.sh also adds a user-level app at `~/Applications/Koma.app` that opens the desktop client from Finder, Launchpad, and Spotlight.
- **Linux** — the raw install.sh binary is GUI-featured and needs WebKitGTK 4.1, GTK 3, and libXtst at runtime. install.sh installs the missing shared libraries with apt (Debian, Ubuntu), dnf (Fedora, RHEL family) or pacman (Arch and derivatives). apt picks whichever package name exists (`libgtk-3-0` or `libgtk-3-0t64`), and dnf resolves each library by its soname. On Arch it runs `pacman -S` without `-y`, so a stale package database gets a printed `pacman -Syu` command instead of a partial upgrade. It also picks the GUI window for the whole machine and writes it to `/etc/koma/gui.conf`: Debian and Ubuntu keep the transparent window with rounded corners, other distros get an opaque window, because WebKitGTK 2.54 on a compositing desktop paints a transparent window only under the cursor. Override with `--gui-window=transparent` or `--gui-window=opaque`, and `koma doctor` shows the mode in use. The `koma` launcher still checks for them and prints an install command if they are missing. install.sh also installs a **user-level** `.desktop` entry + icons under `~/.local/share` so Koma appears in the app drawer (useful on immutable/atomic desktops like Fedora Bluefin where `.deb` is awkward). If you prefer a package that handles dependencies automatically, grab the `.deb` or `.AppImage` instead; both are `gui`-featured and add a system desktop entry that launches straight into `koma gui`. Prebuilt Linux binaries target **glibc 2.35 (Ubuntu 22.04 LTS)** and run on 22.04 and newer, current Fedora and Arch included.
- **Linux servers** — `curl -fsSL https://koma.run/install.sh | sh -s -- --headless` (or `KOMA_HEADLESS=1`) installs `koma-linux-<arch>-headless`, built without `gui`. It needs no GTK or WebKit, no sudo, and adds no app-list entry. `koma remote` does not need either install: on connect it downloads the headless build matching the client into `~/.koma/remote/koma` on the server, and leaves any Koma the user installed alone. Releases published before the headless builds only have the desktop build, so for those both fall back to it.
- **Windows** — `irm https://koma.run/install.ps1 | iex` installs the `.msi` (GUI build, plus a fallback shell so agent commands and the built-in terminal are not stuck on `cmd`). The MSI does not bundle the WebView2 runtime. Windows 11 and most patched Windows 10 machines already have it; if it is missing, `install.ps1` first installs Microsoft's Evergreen WebView2 runtime silently. If you double-click the MSI instead, WebView2 is whatever the MSI's own check finds. If Git for Windows is installed, koma uses that Bash instead of the bundled one. Security tools and full-internet/research mode are not supported on Windows.

Grab the platform installer from the [latest release](https://github.com/aula-id/koma/releases/latest): `koma-x64.deb` / `koma-arm64.deb`, `koma-x64.AppImage` / `koma-arm64.AppImage`, `koma-x64.msi`.

Building the GUI from source yourself (e.g. a gui-featured Linux binary):

```sh
python3 scripts/prepare-terminal.py
cargo build --locked -p agent --features gui
```

Linux build prerequisite: `webkit2gtk-4.1` and its dev headers (`libwebkit2gtk-4.1-dev` on Debian/Ubuntu, `webkit2gtk4.1-devel` on Fedora, `webkit2gtk-4.1` on Arch), plus GTK3 and libsoup3. macOS and Windows use the system-provided webview (WebKit / WebView2) — no extra deps. Any build with the `gui` feature also needs Node and npm on `PATH` (the Rust build script runs `npm install` and `npm run build` in `src-webgui`; run `nvm use 24` first). The default build enables the GUI; use `--no-default-features` for a terminal-only build. The complete terminal payload is included in both.

More at **[koma.run](https://koma.run)**.

---

*Curious how it works under the hood? See [`ARCHITECTURE.md`](ARCHITECTURE.md).*

*Extending the desktop GUI? See [`WEBGUI_SIDEBAR_TABS.md`](WEBGUI_SIDEBAR_TABS.md).*

*Adding a new GUI feature? See [`ADDING_GUI_FEATURES.md`](ADDING_GUI_FEATURES.md).*

*Connecting GUI tabs to Rust? See [`WEBGUI_IPC.md`](WEBGUI_IPC.md).*
