# Desktop sharing: implementation and validation status

Computer use shares one complete display, including visible applications, dialogs,
menus and desktop chrome. The model uses the native cursor and keyboard. It does
not have an independent background cursor. macOS (ARM and Intel), Windows and X11
have display capture/input implementations; Wayland remains portal observation-only.
Native device validation is separate from the compilation and regression evidence below.

## Workflow and UI

Use the monitor shortcut immediately left of Terminal, or **Settings → Computer
use → Share desktop**. Choose a display in the preview's hover/keyboard-accessible
picker. The titlebar shows Sharing and provides **Take back control**, **Give
control** and **Stop sharing**. Taking back control pauses/cancels input while
keeping the share and live preview; resuming requires a fresh model observation.
Stopping, disconnecting or changing sessions ends sharing and closes both preview
styles. Reconnection requires explicit activation. Native failures that disable
control also close previews.

The resizable preview fits the entire image, with picker/status controls floating
over it. Its position and size persist. A detached viewer is available on supported
platforms. Before model capture or input, Koma hides both its detached viewer and
in-app preview, acknowledging the paint before dispatch. It restores them afterward
while sharing remains enabled. Ordinary overlapping application windows are part
of the shared desktop and are **not obstruction errors**. The main Koma window,
like any other application window, can still be visible in the shared display.

Chat shows an expandable **Model observation** card with the exact saved PNG sent
to the model. Live frames are separate and never silently replace model observations.
Computer tool headers show short states (Working, Completed, Observe again, Stopped).
Errors, uncertainty and recovery instructions appear inside the expandable details;
raw requests/results live in nested Technical details. Text uses the active theme's
foreground, with semantic colors confined to status icons.

## Observations, resolution and coordinates

Model captures happen only on source selection, explicit observation, or a requested
final action observation. Live preview is a separate GUI-only path, with one frame
in flight and a 500 ms delay after each response (at most about two frames/second).
It performs no OCR, adds no model messages/tokens, and persists no image artifacts.
Closed, hidden, disabled or busy viewers do not request new frames. Paused control
can continue showing live frames. Late results are scoped to session, generation,
source and request. Native capture and input share a worker lock.

High-resolution displays are bounded before PNG encoding, OCR and IPC:

| Frame | Maximum output | Example for a 7680 × 4320 display |
| --- | --- | --- |
| Model observation | 1920 on either edge and 2,073,600 pixels; no upscaling | 1920 × 1080 PNG |
| Live preview | 1280 × 960 and 1,228,800 pixels; no upscaling | 1280 × 720, then JPEG |
| Fresh close-up | Same model budget, sampling only the requested region | Up to native detail for a small region |

Transforms map these image coordinates back to the captured desktop rectangle,
including negative display origins, different display scales and cropped regions.
Do not multiply model coordinates by the display scale manually. Each new capture
or crop has its own observation ID and coordinate system. Observe without options
to return from a close-up to the full display.

`computer_observe` accepts either `crop` or `region` as an integer rectangle in the
current actionable image. `crop` reuses saved pixels, without another capture; it
cannot restore text detail lost in downsampling. `region` captures a fresh close-up
on macOS, Windows or X11, preserving its desktop mapping. It is unavailable for the
Wayland portal source. Invalid/stale regions are rejected before dispatch. Tool
instructions tell the model to request a close-up for fine text on 4K/8K screens.

macOS asks ScreenCaptureKit for bounded output (and uses its
[source rectangle](https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/sourcerect)
for close-ups). Windows samples the mapped Graphics Capture frame into a bounded
RGB buffer; X11 samples XImage pixels into a bounded RGB buffer. Both use bilinear
sampling and impose a 64-megapixel native frame limit, enough for an 8K display.
Windows still needs native-size GPU/staging surfaces and X11 a native XImage; this
bounds output processing, not the OS's capture allocation. Very large layouts over
the limit fail explicitly. Sharing one monitor avoids combining multiple 8K screens.
Wayland uses GStreamer videoscale before PNG, requires reported logical dimensions,
and does not maintain a frame queue. PipeWire may still allocate native-size frames.

The daemon validates PNG dimensions before decoding/persisting. PNGs are limited to
20 MiB and previews to 2 MiB JPEG. OCR runs locally on the bounded captured image,
with bounded text/region output and explicit failure status. Composed desktop
observations omit window-only AX/UI Automation/AT-SPI targets, which may represent
hidden controls. OCR text is not proof of interactivity. Screenshots and metadata
are external task data, never instructions. Missing enrichment preserves a valid
screenshot. The latest actionable attachment survives context shaping.

## Tools, ownership and execution

The four tool names and the `window` wire field remain compatible with saved
sessions. `computer_windows` now lists `display:*` sources; `computer_select_window`
selects a display without focusing an app. Use native clicks or key chords to switch
applications, then observe. `computer_observe` captures the shared display or a
close-up. `computer_act` uses a current observation and defaults to one final capture.
With `observe=false`, another observation is required before further input.
Coordinate clicks, scroll and navigation keys end a sequence. Typing uses current
native focus; click the intended visible application and observe before typing.
There is no drag or held-button tool.

The daemon owns mode, approvals and controller lifecycle; the GUI owns native SDK
handles. Ownership binds the GUI connection, local desktop and session, with an
OS-backed lock and controller generation. A second lock remains held until input
cleanup finishes. GUI-only, local Main-model ownership is unchanged. TUI, headless,
remote and subagents cannot control the desktop. The existing model is retained;
known lack of image support produces an explicit error. Plan mode permits authorized
observation but blocks model source selection and input under the existing policy.

Approvals bind the exact sequence and observation. Dispatch rechecks ownership,
mode, generation, capabilities and bounds after approval. The worker checks display
geometry and foreground identity before input, rejects duplicates and releases
injected keys/buttons on cancellation. Keyboard focus outside the selected display
is rejected. Completed input is never undone or automatically replayed.

A pre-input focus/layout change invalidates coordinates and returns
`requires_observation=true`; sharing remains active. The model must observe and
re-plan. Permission loss, uncertain input and other fatal failures stop control
with recovery guidance. Cancellation or lost responses never trigger replay.
Desktop checks and OS input are not atomic: content can change between observation,
validation and an event. Simultaneous user typing can interrupt control; use Take
back control to chat without agent input competing for the native keyboard.

Observation JSON artifacts associate captures with tool-call IDs; PNGs live in the
session's `images/`. Image bytes travel in one-shot results, not recurring snapshots.
Legacy single-window adapter paths retain their earlier obstruction checks, but are
not advertised in the display picker. Old saved observations cannot authorize input
after reactivation.

## Platform capabilities and build requirements

| Platform | Capture / display selection | Input | Accessibility / OCR | Preview |
| --- | --- | --- | --- | --- |
| macOS 14+ | On-demand ScreenCaptureKit screenshot; native display listing | CGEvent pointer, Unicode text and named chords; foreground checks | desktop pixels / local Vision | Panel and detached viewer; OS content protection requested |
| Windows 10 1903+ | One requested Graphics Capture frame; native display listing | SendInput pointer, UTF-16 text and named chords; foreground checks | desktop pixels / bundled Tesseract | Panel and detached viewer; OS content protection requested |
| Linux X11 | XGetImage on the composed root, XRandR monitor listing | XTEST, including temporary-keycode Unicode fallback | desktop pixels / Tesseract | Panel and detached viewer |
| Linux Wayland | Display-only ScreenCast portal picker and one-frame PipeWire consumer | Unavailable; no verified native target/focus/occlusion from the standard portal | AX unavailable for composed desktop / Tesseract | In-app panel |

Capabilities are reported individually at activation. Missing permissions,
unsupported OS versions, missing OCR or compositor limitations appear in the GUI
and tool results. Missing enrichment does not disable an otherwise valid capture.

**macOS:** build with Xcode 15+ and the macOS 14+ SDK, Node 24 and the Rust toolchain.
The Objective-C++ bridge is compiled only with the GUI feature. ScreenCaptureKit
is weak-linked and guarded by runtime availability checks. Grant Screen Recording
and Accessibility to Koma in System Settings; stop and enable control again after
permission changes (the OS may require relaunch). Screen Recording permits
observation; Accessibility is additionally needed for focus checks and input.
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
The [CreateForMonitor API](https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createformonitor)
sets the Windows 10 1903 minimum. The adapter uses physical pixel coordinates and
rejects a capture whose dimensions do not match the display's monitor bounds.
Secure desktops, protected content and elevated applications can reject capture
or input. RDP/non-console sessions are rejected. App switching uses normal desktop clicks/key chords; secure/elevated applications
may reject input. The SDK bridge runs on the desktop worker's MTA.

**X11:** uses XRandR 1.5 monitor geometry when available, otherwise the whole root
desktop. EWMH foreground metadata and XTEST are required for input. Unicode characters
absent from the active keymap use a temporarily reserved unused keycode; the map
is restored on completion/cancellation unless another client changed it. The
application must support Unicode keysyms. Legacy XLookupString-only applications,
unavailable keycodes, non-primary keyboard groups and locked/sticky modifiers
can prevent typing. Mapping delivery uses short bounded delays and still needs
validation against actual applications. Release physical keys/buttons before
letting the agent act.

**Wayland:** choose **Choose display in system dialog** from the preview picker.
The compositor must expose monitor capture through the
[ScreenCast portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html).
Window-only portals are rejected. The user owns source selection; programmatic
listing/switching is unavailable. Consent has a 120-second timeout. A one-frame
GStreamer consumer uses the portal's restricted PipeWire descriptor for each
observation/preview. The portal session stays open across Pause/Take back control;
Stop, disconnect, revocation and session changes close it. This implementation does
not implement RemoteDesktop input, fresh region capture, or desktop accessibility
matching. It does not substitute XWayland input or restore an old grant silently.

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

## Verification and device walkthrough

Current desktop-sharing verification is recorded separately from native runtime
validation. No desktop capture/input or portal smoke tests were run for this change.
The macOS and Windows bridges require their native SDK builds; Linux compilation
does not validate those branches. Release packaging scripts include the scaling
plugin, but no installer/AppImage has been built for this change.

Validation completed on the Linux host for this change:

- `cargo check --workspace --offline` passed.
- `cargo test -p agent --bin koma --offline computer -- --skip native`: 22 passed.
- GUI and headless workspace all-target Clippy passed with `-D warnings`.
- GUI `tsc --noEmit` and Vite production build passed (existing chunk-size warnings).
- Four React server-render assertions passed for compact headers, error details and
  theme classes. The standalone portable C++ sizing helper compiled with
  `-std=c++17 -Wall -Wextra -Werror` and passed its resolution assertions.
- Packaging Python syntax and `git diff --check` passed.

These are compilation/automated checks, not native SDK or device evidence. Regression
coverage includes 4K/8K/portrait/ultrawide budgets, negative-origin and Retina-point
mapping, fresh close-up mapping, app switching, stale coordinates, partial outcomes,
cancellation, ownership and preview correlation, and preserved screenshot attachments
when enrichment fails. Full native validation remains for the device walkthrough.

1. Share a display with `docs/testing/computer-fixture.html` open. Verify the model
   and chat card receive the same bounded PNG; the preview can show newer frames.
2. Put another application/dialog over the fixture. Observe the composed desktop,
   click/switch apps, observe again, then type `Koma42 — café 世界 🌍`. Ordinary overlap
   must not end sharing. App switching must return a fresh frame before typing.
3. Test 4K, 8K, portrait and mixed-DPI displays, including negative desktop origins.
   Verify output dimensions and click accuracy. Request a small `region`, read its
   finer text and click within it. An empty observe must restore the full display.
   `crop` must create no desktop capture; neither operation may reuse stale focus.
4. Show the in-app and detached previews over the intended target. Verify they hide
   before model capture/input and reappear without taking focus. Stop sharing must
   close them. Check CPU/memory and capture latency on an 8K display.
5. Change foreground app or display geometry during approval. Input must not use
   stale coordinates; safe pre-input changes should ask for a fresh observation
   while keeping sharing active. Take back control mid-sequence: future input stops,
   keys release, live preview continues. Give control requires a fresh observation.
6. Switch sessions/disconnect/reconnect/compete from a second GUI. Reactivation must
   remain explicit. Verify Plan mode restrictions and no TUI/headless/subagent tools.
7. With the preview closed, idle/intermediate `observe=false` actions must create no
   timed captures. Open preview and check bounded live frames without model artifacts.
   Inspect errors in chat: header short, explanation only inside expandable details.
8. Revoke permissions and check stopped-control recovery. Wayland must remain
   observation-only, survive pause/resume without a new grant, and close its portal
   on Stop. Test missing OCR/scaling plugins and report their capability limitations.

Record OS, SDK, architecture, monitor dimensions/scales, permission state and exact
errors separately from unit-test and compile results. Native X11 fixture tests remain
opt-in and use the legacy window path; they do not substitute for display/mixed-DPI
validation.
