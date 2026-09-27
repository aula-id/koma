# Computer use: implementation and validation status

The daemon, model-loop integration, GUI preview and native desktop adapters are
implemented in source. **macOS and Windows still need compilation with their SDKs
and device validation.** Wayland supports portal observations; input is explicitly
unavailable because the standard portal cannot verify window focus and obstruction.
This is an experimental implementation, not a cross-platform validation claim.

## What is implemented

Computer use starts disabled. In a local GUI session, use the monitor shortcut
immediately left of Terminal in the titlebar, or open **Settings → Computer use**
and enable control. The shortcut shows activation progress and stops control when
already enabled. The preview opens only after activation succeeds.
In the preview, hover over the image (or focus the window picker
with the keyboard) and choose a visible window. Pause, Resume, Stop, Take over,
preview opening/detaching, and capability/extraction details live in Settings. GUI selection observes
without changing focus, including in Plan mode. The model can list windows,
focus/select a window after normal approval, observe, and execute bounded input
sequences. The existing Main model remains in charge; an explicitly known
non-image-capable Main model receives an error before observation/input dispatch.

The native local GUI registers its desktop on the connection before activation;
unregistered, unattached and headless clients cannot enable control. The daemon
binds ownership to that desktop, connection and session, holds an OS-backed
lock, and issues a fresh generation when activated, paused, resumed, or stopped.
A second native lock remains held until an executing GUI worker releases input.
The locks conservatively serialize all desktops for one application-data root.
Mode changes, disconnects, session switches, errors, Stop and Take over revoke
control. Reconnection requires explicit activation. Pause cancels queued work;
Resume requires another observation. Cancellation cannot undo completed input.

Model observation screenshots occur only on selection, explicit observation, or a
requested final action observation. An open live preview separately requests bounded
frames, without adding messages, performing OCR/accessibility extraction, changing
actionable observations, or consuming model tokens. `computer_act` defaults to `observe=true`. `observe=false`
invalidates the preceding observation. The next action needs a new observation.
Crops reuse a saved image and retain its desktop-coordinate transform. Scroll and
key chords terminate a sequence. A click followed by further input requires an
accessibility target with an editable-field role; coordinate clicks terminate
the sequence because they may navigate. A typed Return/Tab must be the final
character, and a chord contains modifiers followed by one key; neither can hide
follow-up input after navigation. Drag and held-button actions are absent.

Approvals apply to the exact queued call. Window selection carries the controller
generation from `computer_windows`; actions carry the observation ID. Dispatch
rechecks ownership, mode, generation, capabilities and coordinates after approval.
The GUI worker rechecks geometry/focus before input, rejects duplicate requests,
and releases injected keys/buttons on return or cancellation. It reports completed
action counts and uncertain failures without replay. Native errors revoke control.
Stopped tool results include explicit recovery guidance: report the failure, stop
the desktop task and wait for the user to resolve it and re-enable control. They
tell the model not to substitute browser/shell input or infer an obstruction's
identity from a visible toast. The chat tool card shows the user-facing recovery
step outside its collapsed technical details. This is model guidance, not a new
global restriction on unrelated browser or shell tasks.

The preview position and size are saved as GUI preferences. Both in-app and detached
previews fit the complete image edge to edge, preserving its aspect ratio when
resized. The window picker and status float over the image on hover or keyboard
focus, without reserving header/footer space. The in-app resize handle supports
pointer dragging and arrow keys (Shift for larger steps). The
floating Computer launcher and extraction overlays have been removed. The optional
native viewer uses the same live frame path and palette. Both previews close when
control stops, fails, disconnects, or changes session; retained observations remain
available in chat. Re-enabling requires an explicit user action.
Before input or observation, the GUI hides the detached viewer and acknowledges that
step before the native worker can start. It restores the viewer after the operation
only while control remains enabled.
Window listing leaves the picker visible. This does not bypass target obstruction checks.

Live preview uses a separate GUI-only request/result channel, with one capture in flight
and a 500 ms interval after each successful response (at most about two frames/second,
slower on expensive captures). Native capture and input are serialized and share the
existing OS lock. Frames are resized to at most 1280 × 960, encoded as JPEG, delivered
only to matching session/generation/window/request IDs, and never persisted. No new
frames are requested by closed, hidden, paused, disabled, or busy viewers. An in-flight
frame may finish after a viewer closes, but cannot become a model observation. A stopped
or switched controller discards its late frames. macOS/Windows skip enrichment for live
frames; X11 still requires unobstructed source windows; Wayland reuses its approved
portal source. Capture failures show a labelled last-model-frame fallback. This is a
low-rate live preview, not a video encoder or independent background desktop.

Control still uses the real OS cursor and keyboard focus. The model can switch existing
windows with `computer_windows` and `computer_select_window`, observing after each
switch before acting. Simultaneous user typing can change focus and interrupt control.
Independent background mouse/keyboard control remains outside this implementation.

The exact saved PNG attached to the model is shown in an expandable **Model observation**
chat card, including its capture time, dimensions and extraction details. Live preview
frames are labelled separately and never replace that saved model image. The full
observation payload remains in model history; the GUI projection omits raw JSON by
default and computer tool rows show readable summaries with expandable details. JSON
metadata under the session's `computer/` directory associates observations with
tool-call IDs; PNGs live under that session's `images/`. Image bytes travel only in
one-shot operation results, never recurring session snapshots. Context shaping
retains the latest actionable attachment. Desktop content is labelled as external
task data. GUI presentation does not change the PNG or model payload.

Accessibility extraction is limited to the selected window. macOS matches AX
windows by process, title and geometry; Windows starts UI Automation at the
selected HWND; X11 matches AT-SPI windows by process, title and bounds. Traversal
is bounded to 256 nodes, 12 levels and a 750 ms traversal budget, with 100 ms
provider-call timeouts. No value/text interfaces are read, and protected controls
are omitted. Provider calls and root matching can add to the traversal budget.

Local OCR operates on the captured PNG/image: Vision on macOS, English Tesseract
on Windows/Linux. It has a two-second budget and at most 256 regions/words. Both
sources identify their provenance; OCR confidence is normalized to 0..1. Failure
returns the valid screenshot with an explicit component status. OCR text does not
establish interactivity and cannot be used as an accessibility click target.

## Platform capabilities and build requirements

| Platform | Capture / window selection | Input | Accessibility / OCR | Preview |
| --- | --- | --- | --- | --- |
| macOS 14+ | On-demand ScreenCaptureKit screenshot; native window listing | CGEvent pointer, Unicode text and named chords; AX focus | AX / local Vision | Panel and detached viewer; OS content protection requested |
| Windows 10 1903+ | One requested Graphics Capture frame; native window listing | SendInput pointer, UTF-16 text and named chords; foreground activation | UI Automation / bundled Tesseract | Panel and detached viewer; OS content protection requested |
| Linux X11 | XGetImage on a visible EWMH window, local Unix-socket display | XTEST, including temporary-keycode Unicode fallback | AT-SPI / Tesseract | Panel and detached viewer; obstruction checks |
| Linux Wayland | Window-only ScreenCast portal picker and one-frame PipeWire consumer | Unavailable; no verified native target/focus/occlusion from the standard portal | AX unavailable for opaque portal identity / Tesseract | In-app panel |

Capabilities are reported individually at activation. Missing permissions,
unsupported OS versions, missing OCR or compositor limitations appear in the GUI
and tool results. Missing enrichment does not disable an otherwise valid capture.

**macOS:** build with Xcode 15+ and the macOS 14+ SDK, Node 24 and the Rust toolchain.
The Objective-C++ bridge is compiled only with the GUI feature. ScreenCaptureKit
is weak-linked and guarded by runtime availability checks. Grant Screen Recording
and Accessibility to Koma in System Settings; stop and enable control again after
permission changes (the OS may require relaunch). Screen Recording permits
observation; Accessibility is additionally needed for labels, focus and input.
The implementation uses Apple's [SCScreenshotManager](https://developer.apple.com/documentation/screencapturekit/scscreenshotmanager)
for explicit observations, without maintaining a capture stream.

Darwin builds support `aarch64-apple-darwin` (Apple Silicon, primary) and
`x86_64-apple-darwin` (Intel). From the repository root:

```sh
# Apple Silicon; also the native default on an ARM Mac.
cargo build --release -p agent --bin koma --target aarch64-apple-darwin
# Intel, including cross-compilation from an ARM Mac with Xcode installed.
rustup target add x86_64-apple-darwin
cargo build --release -p agent --bin koma --target x86_64-apple-darwin
```

The workspace `.cargo/config.toml` defaults `MACOSX_DEPLOYMENT_TARGET` to `11.0`
for both architectures and every C/C++ dependency as well as Rust. An existing
environment setting overrides the default; use the same value for the whole
build. This is the build baseline, not a change to computer observation's macOS
14 minimum. When invoking Cargo from outside this checkout, set the variable
explicitly because Cargo discovers configuration from the working directory.
If old objects still produce newer-deployment-target warnings, rebuild with a
fresh `--target-dir target/darwin-rebuild` instead of reusing those objects.

The bridge ABI is exported through `agent::computer_native` in the library target.
Cargo applies [build-script native library directives](https://doc.rust-lang.org/cargo/reference/build-scripts.html#rustc-link-lib)
to that target; the executable imports it so the archive, C++ runtime and SDK
frameworks reach the final link. Separate GUI release-link CI builds now cover
ARM and Intel. These compile and link the executable without opening a desktop;
`cargo check` and headless CI alone cannot verify this path.

**Windows:** build with the MSVC Rust target, Visual Studio C++ tools and a recent
Windows 10/11 SDK containing C++/WinRT and Graphics Capture headers. GNU builds
remain available with `--no-default-features`; the GUI SDK bridge requires MSVC.
The [CreateForWindow API](https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow)
sets the Windows 10 1903 minimum. The adapter uses physical pixel coordinates and
rejects a capture whose dimensions do not match the window's DWM frame bounds.
Secure desktops, protected content and elevated applications can reject capture
or input. RDP/non-console sessions are rejected. Foreground activation can be
refused by Windows: activate the target manually and select it for observation in
the GUI. UI Automation runs on the desktop worker's MTA.

**X11:** requires EWMH window/PID metadata and XTEST for input. Unicode characters
absent from the active keymap use a temporarily reserved unused keycode; the map
is restored on completion/cancellation unless another client changed it. The
application must support Unicode keysyms. Legacy XLookupString-only applications,
unavailable keycodes, non-primary keyboard groups and locked/sticky modifiers
can prevent typing. Mapping delivery uses short bounded delays and still needs
validation against actual applications. Release physical keys/buttons before
letting the agent act.

**Wayland:** enable Computer use in Settings and choose **Choose source in system dialog**
from the preview window picker. The
compositor must expose window capture via the [ScreenCast portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html).
Monitor-only portals are rejected. The user owns source selection; listing and
programmatic switching are unavailable. The source-picker request allows up to
120 seconds for consent. GStreamer consumes one frame for each model observation
or live preview request, using the portal's restricted PipeWire file descriptor.
The portal session stays open; frames are consumed only for requested observations
or while the user has an active live preview open.
Pause, Stop, disconnect and session changes close it; resuming requires choosing
the source again. There is no XWayland fallback or automatically restored grant.
The portal's opaque source identity is insufficient for selected-window AT-SPI
matching or the input contract, so this implementation deliberately exposes
neither. A future compositor-specific adapter would be needed for verified input.

Native adapters reject input when another window (including Koma's preview)
obstructs the selected target. macOS/Windows window capture may still observe an
occluded target. X11 also rejects occluded capture. Native windows are configured
not to steal focus; if a desktop focuses the detached viewer, Koma pauses and
falls back to the panel. Content-protection/exclusion support depends on the OS.

Desktop validation and OS input are not atomic: focus/content can change between
a check and an input event. Bounds/title checks cannot detect every same-window
content change. Native window identifiers also retain a rare same-process window-ID reuse
limitation, even when combined with process identity. No timeout or lost response triggers an
automatic input retry. Cancellation prevents future input and releases injected
keys/buttons on a best-effort basis; it does not undo completed actions.

## Release packaging

OCR and portal dependencies are confined to GUI builds/packages. Raw development
and custom builds may use installed Tesseract from PATH; missing English data or
OCR failures are reported with the screenshot. macOS uses OS-provided Vision and
ships no Tesseract runtime.

The existing Windows WiX installer now includes `ocr/tesseract.exe`, its DLLs,
English data, configuration and upstream notices. Run from the repository root:

```powershell
./scripts/prepare_windows_ocr.ps1
cargo build --release -p agent
cargo packager --release --formats wix --verbose
./scripts/check_windows_ocr.ps1
```

Preparation downloads a pinned upstream Tesseract installer, verifies its SHA-256,
installs it into a temporary staging directory, and generates the WiX fragment
under `packaging/windows/`. The fragment is generated per checkout and ignored by
Git. The verification script administratively extracts the actual MSI and runs
its packaged OCR against a synthetic blank image. The existing WiX template and
version scheme are unchanged.

Linux Debian metadata requires Tesseract/English data, AT-SPI, XTEST, the desktop
portal and GStreamer/PipeWire packages. The user's desktop must supply the matching
portal backend. AppImage staging includes the Tesseract runtime/data/notices,
XTEST, GStreamer launcher/plugins/scanner, and PipeWire client modules/SPA plugins
and configuration. PipeWire's dynamically loaded modules need explicit staging in
addition to linked libraries; its [runtime path settings](https://docs.pipewire.org/page_man_pipewire_1.html)
are applied only to the capture subprocess.

Install the packages named in the release workflow, then run from the repository
root on the native Linux packaging architecture:

```sh
python3 scripts/stage_linux_portal.py
cargo build --release -p agent
cargo packager --release --formats deb,appimage --verbose
python3 scripts/check_computer_package.py target/release
```

The release workflow runs these preparation and artifact checks. The ARM64 package
job uses the native `ubuntu-22.04-arm` runner listed in the
[GitHub runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners),
so the executable and staged libraries match the advertised architecture.
The AppImage check executes bundled OCR and loads bundled GStreamer plugins; it
does not open a portal or capture a desktop. These gates are source implementation,
not evidence that a release artifact has been built or validated in this session.

## Verification evidence

For this continuation, the user requested implementation and static checks only,
with device testing left to them. No native smoke test, desktop capture, input
injection, new automated test-suite execution, installer build or package execution
was performed. The macOS/Windows bridges have been source-reviewed but **have not
been compiled** here because the corresponding SDKs are unavailable.

Linux workspace checks and Clippy with `-D warnings` passed with default GUI
features and with `--no-default-features`. The GUI TypeScript check and Python
packaging-script syntax checks also passed. Commands used:

```sh
cargo check --workspace --offline
cargo check --workspace --no-default-features --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo clippy --workspace --no-default-features --all-targets --offline -- -D warnings
# From src-webgui:
npx --no-install tsc --noEmit
```

The existing fake-desktop coverage checks capture counts, select/click/type/observe,
partial execution and cleanup, stale focus/geometry/title, cancellation,
controller contention, duplicate/late replies, generation changes after approval,
model/preview byte identity, crop transforms, enrichment failure and tool dispatch.
These tests are compiled by Clippy's `--all-targets`; compiling is not executing.

Historical evidence from the preceding X11-only implementation (before this
continuation): 1,817 workspace tests passed, two opt-in native tests were excluded
from that suite, and the headless computer subset passed 10 tests. The two native
tests were subsequently run explicitly on 2026-09-27 on X.Org 1.20.14 at 1280×720.
They verified ASCII fixture click/type/capture/cleanup and detached-viewer focus
plus Pause IPC. WebKit emitted a DRI3 warning but the tests passed. Those results
do not validate the new Unicode path, portal adapter, native macOS/Windows bridges,
OCR/AX coverage, mixed-DPI hardware or release packages.

## Live preview and GUI follow-up validation

This follow-up adds the Computer use settings section, image-first observation
cards, hover window selection, separate live GUI frames, and acknowledged hiding
of the detached viewer before native operations. The model still receives only
explicit observations and controls the native cursor/keyboard.

Validation on the Linux build host:

- `cargo check --workspace --offline` passed.
- Workspace and headless all-target Clippy with `-D warnings` passed.
- GUI TypeScript checking and Vite production build passed (existing large-chunk
  warnings remain).
- `cargo test -p agent --bin koma --offline computer -- --skip native` passed:
  18 tests, including preview session/generation/source invalidation, native
  viewer sizing, stopped-control recovery with preserved partial outcomes, and
  compact chat projection through the actual daemon-to-GUI shadow session. That
  projection regression covers shadow sessions without filesystem paths, which
  previously caused valid observations to fall back to raw JSON.
- Additional Node assertions passed for 24 portrait/landscape viewport sizing
  cases, activation/stop/session-switch preview visibility, and static React
  rendering of observation cards, visible recovery guidance and image overlays.
- `git diff --check` passed. Full-workspace `cargo fmt --all -- --check` still
  reports formatting differences, including unrelated untouched files; no
  repository-wide formatting rewrite was applied.

No native desktop, input, focus, or portal tests were run for this follow-up.
The modified macOS/Windows capture branches still need builds with their platform
SDKs, and the live feed/viewer visibility behavior needs device verification.

## Device walkthrough

Open `docs/testing/computer-fixture.html` locally in a browser. Start a local Koma
GUI session, enable Computer use in Settings and choose that window in the preview
(system picker on Wayland).
Use the existing Main model with image input and follow normal action approvals.

1. Observe and expand the Model observation chat card; compare its image against
   the exact saved model attachment. Inspect AX/OCR statuses in Settings. Confirm that the
   password value is absent from extracted metadata. Crop an observation and
   confirm that the original capture count does not increase.
2. On input-capable platforms, select/focus through the model, click the Message
   field and type `Koma42 — café 世界 🌍`. Use an AX editable target for a combined
   click/type sequence, or observe after a coordinate click before typing. Click
   Apply in a separate sequence; the counter must increment once and text match.
   Try Ctrl+A (Command+A on macOS), Backspace and Return/Tab as separate final keys.
3. Scroll the fixture in its own sequence. Resize/move the target, including
   between differently scaled monitors and a monitor with a negative origin;
   an old observation must be rejected and a new observation must map correctly.
4. Place the detached preview over the target: verify it hides before an operation
   and returns afterward without taking focus. Obstruct with a different window
   and verify input refusal, preview closure, and stopped-control recovery in chat.
   Verify the model asks for help rather than guessing the blocker or switching
   to browser/shell tools. After explicit reactivation, a new observation is required.
   Switch focus or close the target during an approval and verify no subsequent
   input. Pause/Stop/Take over during a sequence; verify no replay and no stuck keys.
5. Switch sessions, disconnect/reconnect and compete from a second GUI. Explicit
   reactivation must be required. In Plan mode allow observation but reject model
   focus/input. TUI, headless, remote and subagent paths must not gain control.
6. Verify no new PNG artifacts while idle or during intermediate `observe=false`
   input. A selection/final requested observation should add exactly one capture.
   With the preview open, change content in the shared window and verify live
   updates without new conversation messages or PNG artifacts. Close the preview
   and verify capture requests stop. Missing OCR/AX must leave model screenshots
   available with a component limitation. Resize landscape and portrait previews:
   the complete image should fill the viewer, with picker/status hovering above
   it instead of blank header/footer bands. Stop, fail or disconnect control and
   verify both preview types close. The titlebar monitor shortcut beside Terminal
   must enable/stop control and open the preview only after successful activation.
7. On macOS revoke Screen Recording/Accessibility and reactivate; on Windows try
   an elevated/protected target. On Wayland cancel the source dialog, revoke its
   sharing grant, pause/resume and choose a replacement source. Input must remain
   unavailable and monitor-only selection must never silently replace a window.

Record OS/build/desktop versions, window scaling, permission state, component
statuses and exact errors separately from compile/lint results. Opt-in X11 tests
remain available for local execution when desired:

```sh
cargo test --bin koma native_fixture_round_trip -- --ignored --nocapture
cargo test --bin koma native_viewer_preserves_focus_and_routes_controls -- --ignored --nocapture
```
