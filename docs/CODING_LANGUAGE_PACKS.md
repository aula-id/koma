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
from PATH. Installing an adapter does not install project dependencies. PHP still
requires Xdebug enabled for the selected PHP runtime; Bash requires bashdb.
Nix and data formats have no DAP adapter.

Runtime recipes support apt, dnf, pacman and zypper on Linux, existing Homebrew
on macOS, and selected winget packages on Windows. Linux uses a desktop pkexec
prompt when available, otherwise sudo in the task's interactive terminal. The
review lists exact packages and agreement flags before installation. Distribution
repositories may need enabling. Apt Rust includes build-essential/pkg-config;
Windows Rust and C++ recipes include Visual Studio Build Tools with the C++
workload and recommended SDK components. Apt PHP includes php-xdebug and pacman
PHP includes xdebug; Linux/Homebrew Bash recipes include bashdb. These are part
of **Include runtime / compiler**, not silent installations. Xdebug activation,
macOS SDKs, unsupported package repositories and other PHP distributions can
still require additional setup. Nix and unavailable native Windows runtimes use an existing
executable. Unsupported recipes fail explicitly without starting installation.
ZLS provisioning reads the selected Zig version and chooses a stable ZLS release
with the same major/minor version. Development/unparseable Zig versions or a
missing compatible release fail explicitly and preserve the previous pointer.
Debugpy's managed environment uses the selected Python executable. Execution
compatibility remains a native acceptance check. Downloads use HTTPS, bounded archive extraction, and release
SHA-256 verification when supplied by the publisher. Unsupported archive links are
rejected. Failed standalone updates preserve the previous component pointer;
unused version directories are retained. Existing legacy LSP installers keep their
previous update behavior.

Workspace executable selection writes `toolchains.<language>.executable` in
`.koma/coding.json`, guarded by the configuration fingerprint. Blank restores automatic
selection (project Python virtualenv, then host PATH). Python candidates include `.venv` and `venv`. Global `environment` and
per-toolchain `environment` maps are passed to new tasks and LSP processes; the
selected executable directory is prepended to PATH without mutating global state.
Environment selection restarts workspace language servers and reopens mounted
documents. The command palette also exposes Restart Workspace Language Servers.
PHP offers Intelephense or Phpactor on Unix; save the choice with Use for workspace.
Test Explorer uses the selected Python interpreter, a project virtualenv, or the
managed debugpy/pytest environment in that order. Project virtualenvs must provide
their own pytest dependency.
Selecting an environment refuses to overwrite unsaved coding settings.

No native installations or functional tests were performed by the implementation
agent. Native acceptance should cover update/cancel/failure, simultaneous roots,
remote hosts, interpreter switching, and package-manager elevation.
