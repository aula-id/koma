# Language packs and project environments

Language Packs live in the **workspace BottomPanel** (same dock as Tasks,
Problems, and Language Servers) — not a floating card. Open via:

- Command palette: **Language Packs: Install, Update and Select Environment**
- Status footer **Packs** control (toggles the panel closed when the tab is already open)
- Bottom panel tab **LANGUAGE PACKS**
- Notebook kernel recovery (**Language Packs** action)

## UI design signature (BottomPanel tool dock)

Match `CodingTasks` chrome. Do **not** reintroduce a floating
`absolute inset-x-10 bottom-6 … shadow-xl` dialog for packs.

| Rule | Detail |
|------|--------|
| Shell | `#workspace-bottom-panel` tab content only |
| Title chrome | Owned by BottomPanel tablist (`TASKS · PROBLEMS · LANGUAGE SERVERS · LANGUAGE PACKS`) + maximize/close. Refresh and open-config icon buttons are header actions (`PanelActions` from `PanelChrome`) rendered into the tablist, not a row in the body |
| Body root | `section` with `flex min-h-0 flex-1 flex-col overflow-hidden text-[11px]` — no rounded card, no own `X` |
| Tool rows | One wrapping strip with workspace, language and view controls; thin borders, compact muted controls |
| Density | Separate Environment from Tools; one scrollable content body with 12px horizontal padding, 16px section gaps, maximum width 48rem (`max-w-3xl`) |
| Buttons | `panelButton` / `panelIconButton` from `PanelChrome` (`flex h-6 … rounded px-1.5 text-koma-dim hover:bg-koma-hover`) |
| Selects | `KomaSelect` + `panelSelect` (`h-6 … bg-koma-panel2 pl-2 text-[11px]`) |
| Paths | Hidden behind (?) help, except the editable environment path and reviewed install commands |
| Copy | Short names + status only. Explanations belong in click/keyboard-accessible (?) disclosures, not permanent paragraphs. |
| Errors | compact `role="alert"` strip under tools (or toast for one-shots) |
| Open API | `showCodingPacks()` dispatches a `koma-coding-packs` window event; the mounted `CodingPacks` listener picks the workspace and calls `setBottomPanelTab('packs')` |
| Install progress | Stays on Language Packs after `packApply` (live output + Stop). **Open in Tasks** is optional. |
| Components | Tools always offers **Language server**, with debugger, runtime, and Python notebook kernel where applicable. Aligned checkbox · name · short status · (?) rows. |

```text
BottomPanel
├── tablist: Tasks | Problems | Language Servers | Language Packs
└── packs tab
    ├── header actions (tablist): refresh · open coding configuration
    ├── wrapping row: workspace select · language (bounded field, arrow inside) · Environment | Tools
    └── scrollable body
        ├── Environment: executable + Apply · Manage tools
        └── Tools: checkbox · component · Installed / Not installed / Manual setup · (?)
            └── Review changes → exact commands → Install / update → progress
```

Preserve the dock signature, not its former cramped density. Never put language
selection, installation checkboxes, and a primary action on one crowded toolbar.
Environment is the default view; notebook recovery opens Tools with ipykernel
checked. Keep one primary action per step. Checkboxes choose what to install;
they are not green indicators of installation status. Unmanaged LLVM debugging
shows **Manual setup**, never a checked but disabled installation choice.

Explanations and full installed paths live in (?) disclosures that work by click
and keyboard. Do not turn tool rows into paragraphs or expose managed directory
paths by default. Help stays in flow to avoid clipping in small docks. Exact
commands remain visible during review so short copy does not hide consent.

The language select uses a bounded outer wrapper and a full-width inner control:
never apply a maximum width only to the inner select while its chevron wrapper
stretches across the dock. The workspace and view controls stay outside the
scroll body. Progress stays on Packs; **Open in Tasks** is secondary. All colors
remain theme tokens; use the accent for active chrome and the primary action.

## Notebook kernel (ipykernel)

Python pack plans can include **Notebook kernel (ipykernel)** — a reviewed
`{workspace python} -m pip install ipykernel` step. Progress stays on **Language
Packs** (optional **Open in Tasks**). Koma never silent-pip-installs when a
notebook opens. From a notebook missing-ipykernel strip, **Install ipykernel…**
opens Language Packs with Python + the notebook checkbox already on.

## Behavior

The workspace dropdown binds every action to its host and root. Status
only inspects installed files. Review installation shows the exact command vector;
Install consumes a host-bound plan valid for ten minutes. Progress and cancellation
use Tasks, independent of chat selection. Pack installations are serialized per
host. Existing LSP installers are reused in child processes so stdout cannot
corrupt the SSH coding protocol.

Packs cover Rust, JS/TS, Python, Go, C/C++, PHP, Lua, Zig, Bash, HTML/CSS/JSON
and TOML. Standalone adapters are provisioned for Python (debugpy), Go (Delve),
JS (js-debug), PHP (Xdebug adapter), Bash and Lua. LLVM's `lldb-dap` is detected
from PATH. Installing an adapter does not install project dependencies. PHP still
requires Xdebug enabled for the selected PHP runtime; Bash requires bashdb.
Data formats have no DAP adapter.

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
still require additional setup. Unavailable native Windows runtimes use an existing
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
their own pytest dependency. Notebooks default their kernel interpreter to the
same workspace Python (see `CODING_NOTEBOOKS.md`); the notebook Select Kernel
chip can override per document without changing the workspace default.
Selecting an environment refuses to overwrite unsaved coding settings.

No native installations or functional tests were performed by the implementation
agent. Native acceptance should cover update/cancel/failure, simultaneous roots,
remote hosts, interpreter switching, and package-manager elevation.
