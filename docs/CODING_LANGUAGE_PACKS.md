# Language packs and project environments

Open **Language Packs: Install, Update and Select Environment** in the command
palette. The workspace dropdown binds every action to its host and root. Status
only inspects installed files. Review installation shows the exact command vector;
Install consumes a host-bound plan valid for ten minutes. Progress and cancellation
use Tasks, independent of chat selection. Pack installations are serialized per
host. Existing LSP installers are reused in child processes so stdout cannot
corrupt the SSH coding protocol.

Packs cover Rust, JS/TS, Python, Go, C/C++, PHP, Lua, Zig, Bash, Nix, HTML/CSS/JSON
and TOML. Standalone adapters are provisioned for Python (debugpy), Go (Delve),
JS (js-debug), PHP (Xdebug adapter), Bash and Lua. LLVM's `lldb-dap` is detected
from PATH. Installing an adapter does not install project dependencies or enable
Xdebug in PHP. Bash requires bashdb. Nix and data formats have no DAP adapter.

Runtime installation currently supports Debian-family apt with a desktop pkexec
prompt and macOS Homebrew already installed. Other hosts, headless SSH sessions,
and Zig/Nix runtimes use a selected existing executable. The UI reports unsupported
recipes explicitly. Version compatibility (especially Zig/ZLS) remains a native
acceptance check. Downloads use HTTPS, bounded archive extraction, and release
SHA-256 verification when supplied by the publisher. Unsupported archive links are
rejected. Failed standalone updates preserve the previous component pointer;
unused version directories are retained. Existing legacy LSP installers keep their
previous update behavior.

Workspace executable selection writes `toolchains.<language>.executable` in
`.koma/coding.json`, guarded by the configuration fingerprint. Blank restores PATH
selection. Python candidates include `.venv` and `venv`. Global `environment` and
per-toolchain `environment` maps are passed to new tasks and LSP processes; the
selected executable directory is prepended to PATH without mutating global state.
Reopen all documents for a language to restart its server after interpreter changes.
Selecting an environment refuses to overwrite unsaved coding settings.

No native installations or functional tests were performed by the implementation
agent. Native acceptance should cover update/cancel/failure, simultaneous roots,
remote hosts, interpreter switching, and package-manager elevation.
