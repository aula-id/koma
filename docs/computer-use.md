# Desktop sharing: implementation and validation status

Computer use offers two source groups: **Screens** and **Applications**. A screen
includes visible applications, dialogs, menus and desktop chrome. An application
shares only its selected window, and on macOS, Windows, and X11 that window accepts
real clicks and typing. The click brings the window forward and uses the real pointer.
A programmed point does not teleport: the pointer glides quickly along the whole path,
and an enlarged arrow is drawn over it for that move. The arrow is left out of the
model screenshot. Wayland portal shares stay observation-only.
Native device validation is separate from the compilation and regression evidence below.

## Workflow and UI

Use the monitor shortcut immediately left of Terminal, or **Settings → Computer
use → Start sharing**. Enabling Computer use grants consent for native desktop
source selection and input until paused, stopped, or disconnected. Computer tools
run without the workspace Safeguard classifier or per-action approval prompts in
Normal, Auto, Yolo, Plan and SDLC modes. This does not change shell/file approvals.
Choose a source in the preview's hover/keyboard-accessible picker. It has separate
**Screens** and **Applications** sections, with both category buttons pinned above
the scrollable list. While open, the picker uses the preview's full area so small
previews cannot bury Applications below the screen list. Opening during an active
operation defers source refresh until the worker is available. A Wayland application
is labelled view only. On macOS, Windows, and X11 an application share is labelled
like any other share; selecting it does not raise the window until an input action.
The titlebar uses one icon for choose-source, paused, and sharing, and provides **Take back control**, **Give
control** and **Stop sharing**. Taking back control pauses/cancels input while
keeping the share and live preview; resuming requires a fresh model observation.
Interrupting a turn cancels the click in flight and leaves sharing enabled.
Only the stop control, disconnect, or a session change turns sharing off.
Stopping, disconnecting or changing sessions ends sharing and closes both preview
styles. Selecting another window or screen keeps the current preview picture
until the next frame has decoded. Reconnection requires explicit activation. Native failures that disable
control also close previews.

Before a source is selected, the preview resizes freely on either axis and shows
only its source picker and a small empty-state icon. After the user sizes it, that
width and height stay. A new frame does not resize the window; the picture fits
inside the user's box. The viewport can only shrink a side that would leave the screen.
Its position and size persist. A detached viewer is available on supported
platforms. Before model capture or input, Koma hides both its detached viewer and
in-app preview, acknowledging the paint before dispatch. It restores them afterward
while sharing remains enabled. Ordinary overlapping application windows are part
of the shared desktop and are **not obstruction errors**. The main Koma window,
like any other application window, can still be visible in the shared display.

Chat shows an expandable **Model observation** card with the exact saved PNG sent
to the model. Live frames are separate and never silently replace model observations.
Computer tool headers show short states (Working, Completed, Observe again, Stopped).
An assist-mode action shows **Select a screen**; it does not stop sharing.
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
A model frame large enough to spare an edge carries a ruler in the margin around
the page, plus a light grid on the page. The numbers are outside the capture so
they are not read as part of the UI. They are screen pixels. A window share
prints that window's position on the screen, so the corner is not zero unless
the window is at the screen origin. Click the printed number; it is not added
to the window origin again, and the margin is not part of the click surface.
The `x` / `y` caption is still desktop pixels per screenshot pixel. Do not
multiply a tick by that caption. Region and crop rectangles stay in image
pixels of the page, not of the expanded frame. The live preview is not marked.
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
with bounded text/region output and explicit failure status. Application captures
can include bounded, source-labelled AX/UI Automation/AT-SPI data. Composed desktop
observations omit window-only AX/UI Automation/AT-SPI targets, which may represent
hidden controls. OCR text is not proof of interactivity. Screenshots and metadata
are external task data, never instructions. Missing enrichment preserves a valid
screenshot. The latest actionable attachment survives context shaping.

## Tools, ownership and execution

The four tool names and the `window` wire field remain compatible with saved
sessions. `computer_windows` lists both `display:*` screens and application windows;
each tool result identifies `source_type` and `view_only`. `computer_select_window`
selects either source without focusing an app. Application input is rejected by the
daemon, worker and native adapters, even if an application window has focus. Use native clicks or key chords to switch
applications, then observe. `computer_observe` captures the shared display or a
close-up. `computer_act` requires a current screen observation and defaults to one final capture.
Use the exact `observation_id` returned with the frame (also `observation.id`),
not the top-level tool request `id` or a scene description. Invalid references and
expired observations are rejected before input with `completed=0`,
`uncertain=false`, and `requires_observation=true`; observe and re-plan using the
new ID. Never silently substitute the current frame or replay previous input.
Screen clicks use screenshot `x`/`y` and omit `element`; accessibility metadata is
not needed for coordinate control. Element IDs, when available, must come from
that observation and cannot be combined with coordinates.

During a computer-use conversation, the main model's bash dispatcher rejects
recognized desktop-automation fallbacks (including `osascript`, `cliclick`,
`xdotool`, `pyautogui`, `SendKeys`, and app launchers such as `open -a`,
`xdg-open`, and `Start-Process`) before spawning foreground or background
jobs. This restriction remains after pause/stop/disconnect; normal shell commands
remain available. It is a targeted tool-routing guard, not a sandbox against
arbitrary or obfuscated programs. Native move, click and scroll actions use the
real OS pointer; a rejected call causes no pointer movement.

With `observe=false`, another observation is required before further input.
Up to 16 predicted actions can run in one batch, including coordinate click →
type, keyboard navigation and repeated scrolls. The model should batch only when
confident about the next target; coordinates refer to the starting screenshot.
A click waits about 200 ms before the next step, so a navigation can start
painting. Key chords get a shorter cancellable focus-settling delay. Typed
characters do not. If the frame after a click still shows the page loading,
the model calls `computer_observe` again and continues. It does not end the
turn or stop sharing on a loading page. Native focus metadata is refreshed after each
action without a screenshot. The default is one final observation for the
entire batch. After the last action, macOS, Windows, and X11 wait a cancellable
200 ms, then capture once. That frame is the next actionable observation.
On macOS, Windows, and X11, a foreground title, bounds, or window-identity
change does not interrupt the batch, including a change caused by an earlier
step. The native keyboard check on each platform is only that the focused
window's center is still inside the shared display. The shared screen moving
or changing resolution does not stop the batch: screenshot coordinates are
mapped onto that screen's current rectangle. A display that is not actionable,
or a focused window that has left that display, still stops the remaining
steps and requests observation. Named key
chords are not blocked by Caps Lock or the active layout. On X11, `type` still
refuses before the first character when the group is not primary, Caps Lock is
on, or a modifier is latched, so a later Enter in that batch is not sent.
macOS and Windows inject text as Unicode, so that layout gate is not used
there. A URL or other string is one click and one `type` of the full text,
then Return. A single punctuation mark is also a valid key, so a spelled-out
period does not reject the batch. There is no drag or held-button tool.

### Saved-image inspection

`computer_observe({"crop":{"x":100,"y":80,"width":300,"height":200}})`
inspects the saved current observation on all platforms, including application
and portal shares. `region` requests a **fresh** high-detail capture and only
works on supported native display sources. An unsupported application/portal
region request now returns a concrete crop call and keeps sharing active.

`load_image` also handles saved images independently of desktop control. Use a
session `image_n` or an allowed `path` (including an observation's `image_path`):

```json
{"image_n":1,"crop":{"x":100,"y":80,"width":300,"height":200}}
```

The crop is attached immediately as a new image artifact. The source stays
unchanged; tool details retain source dimensions, crop bounds, the new image
marker/path, and the output-to-source coordinate mapping. Image inspection does
not refresh or authorize a computer observation. Use a current observation and
map coordinates back before computer input.

For exact color values without another attachment:

```json
{"image_n":1,"points":[{"x":120,"y":90},{"x":240,"y":90}],"attach":false}
```

Points always address the full EXIF-oriented source image, even when a crop is
also requested. Results include RGB hex and 8-bit RGBA values sampled before
resizing; alpha is retained separately from RGB hex. These are saved pixel values,
not color-managed screen measurements. `attach:false` without points returns
image dimensions/crop metadata. Inspection accepts at most 64 points, 20 MiB of
encoded input and 32 megapixels of decoded image; attached inspection images are
capped at 1920 pixels per edge and 2 megapixels. It uses the existing Rust image
decoder on macOS, Windows and Linux, without Python or a new native dependency.
The computer-session shell guard redirects common PIL/OpenCV crop and pixel
sampling commands to this built-in path. As with desktop shell fallbacks, this is
a targeted routing guard, not a sandbox for arbitrary scripts.

### Native execution

Key names use one case-insensitive vocabulary across validation and native adapters.
For example, `Command`/`cmd`/`Meta`/`Super` plus `Space`/`space`/a literal space all
resolve to the same Spotlight chord on macOS. `Control`/`Ctrl`, `Alt`/`Option`,
`Shift`, letters/digits, navigation keys and F1–F12 are supported; Windows/X11 also
accept F13–F24 when available in the active keymap. A chord has modifiers followed
by exactly one non-modifier key. Use `type` for text. Unknown names fail before
dispatch without revoking control. Native key lookup is also checked before input,
so an unavailable platform key does not falsely report uncertain injected input.


The daemon owns the controller consent lifecycle; the GUI owns native SDK
handles. Ownership binds the GUI connection, local desktop and session, with an
OS-backed lock and controller generation. A second lock remains held until input
cleanup finishes. GUI-only, local Main-model ownership is unchanged. TUI, headless,
remote and subagents cannot control the desktop. The existing model is retained;
known lack of image support produces an explicit error. Workspace mode does not
restrict explicitly enabled native control, including source selection and input.

Dispatch rechecks active consent, ownership, generation, capabilities and bounds.
The worker checks display
geometry and foreground identity before input, rejects duplicates and releases
injected keys/buttons on cancellation. Keyboard focus outside the selected display
is rejected. Completed input is never undone or automatically replayed.

A pre-input focus/layout change invalidates coordinates and returns
`requires_observation=true`; sharing remains active. The model must observe and
re-plan. Permission loss, uncertain input and other fatal failures stop control
with recovery guidance. Cancellation or lost responses never trigger replay.

On macOS, Windows and X11, physical key/button contention gets a cancellable
250 ms settling window. This also lets queued synthetic key-up events settle
before a subsequent character/action checks the OS key-state table. macOS also
tracks the Unicode carrier key as injected input until typing cleanup, so the
next character cannot classify its own synthetic key-down as physical input. Persistent
contention is reported as `input_busy`. Adapters explicitly track whether the
current action emitted events: if it sent none, the result is `uncertain=false`,
sharing stays enabled, and the model can observe then decide the remaining input.
Earlier completed actions remain completed and must not be replayed. If contention
occurs after input starts, partial input remains uncertain and control stops; the
runtime never guesses that partially typed text can safely be sent again.

The portable settling helper and the Windows newest-frame choice can be checked
without native desktop access:
`c++ -std=c++17 src-agent/native/computer/input_idle_test.cpp -o /tmp/koma-input-idle-test`
then `/tmp/koma-input-idle-test`. Device-level macOS/Windows input still requires
native validation.
When the desktop changes during a requested full-source capture (for example,
after a click brings an app forward), the worker makes at most three capture
attempts with short cancellable settling delays. Only observations are retried;
the action sequence is never replayed. A completed batch waits 200 ms before
that first capture so the result can paint; the wait is skipped for
`observe=false` and for a standalone observe. Region captures remain bound to
their original scene and require a fresh full observation if it changes. If the
scene does not settle, the result retains the completed input count and asks
the model to observe again.

If the model ends a desktop turn with a short action announcement but no tool
call (for example, “Compass is focused — selecting the screen and inspecting it”),
Koma uses the existing two-reminder budget to request the missing tool call.
This covers successful tool rounds as well as observation-recovery failures and
empty completions. The continuation only asks the model to choose its next tool;
it never synthesizes or replays native input. Finished reports, questions and
requests for user help may end normally. The check is scoped to desktop tool use
in the current conversation turn, so leaving sharing enabled does not make later
unrelated chats continue automatically. Exhaustion or an unavailable continuation
client produces a persistent chat notice rather than silently marking the task
complete. A model stream that loses
its producer without a terminal event ends with a visible interruption error;
returned request errors are forwarded to the chat instead of discarded.

Diagnostics in `~/.koma/error.log` include `computer.dispatch`, `computer.rejected`,
`computer.result`, `computer.reply_dropped`, `computer.stop`, `computer.capture`,
`computer.turn` and `turn.error`. Operation entries correlate session/request IDs,
elapsed time, completed count, uncertainty and recovery state. They do not include
action arguments, typed text, screenshots or OCR metadata. Operation diagnostics
also go to the session's `error.log`; `gui-pace` entries alone only show UI delivery.
Desktop checks and OS input are not atomic: content can change between observation,
validation and an event. Simultaneous user typing can interrupt control; use Take
back control to chat without agent input competing for the native keyboard.

Observation JSON artifacts associate captures with tool-call IDs; PNGs live in the
session's `images/`. Image bytes travel in one-shot results, not recurring snapshots.
If the model tries to act on an application share, the result has
`requires_screen=true`, `controller_enabled=true`, and a short recovery instruction:
list sources, select the screen containing the app under the existing GUI consent, inspect
its new observation, and continue the user's task. Do not stop/re-enable sharing or
substitute browser/shell input. Old application observations cannot authorize screen
input. On X11, an occluded application cannot be captured reliably; that observation
also returns the screen-selection nudge rather than disabling sharing. Permission
loss and uncertain native input retain the existing stopped-control recovery.

This nudge is model guidance, not an automatic replay. All source changes and
input require the active local GUI controller; pause and stop revoke that consent.

## Platform capabilities and build requirements

| Platform | Capture / source selection | Input | Accessibility / OCR | Preview |
| --- | --- | --- | --- | --- |
| macOS 14+ | On-demand ScreenCaptureKit screenshot; screen and application listing | CGEvent pointer, Unicode text and named chords; foreground checks | AX for applications / local Vision | Panel and detached viewer; OS content protection requested |
| Windows 10 1903+ | One requested Graphics Capture frame; screen and application listing | SendInput pointer, UTF-16 text and named chords; foreground checks | UI Automation for applications / bundled Tesseract | Panel and detached viewer; OS content protection requested |
| Linux X11 | XGetImage on the composed root, XRandR monitor listing | XTEST, including temporary-keycode Unicode fallback | AT-SPI for applications / Tesseract | Panel and detached viewer |
| Linux Wayland | Screen/application ScreenCast portal picker and one-frame PipeWire consumer | Unavailable; no verified native target/focus/occlusion from the standard portal | AX unavailable for composed desktop / Tesseract | In-app panel |

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
The Applications picker uses ScreenCaptureKit's
[shareable windows](https://developer.apple.com/documentation/screencapturekit/scshareablecontent?language=objc),
the same discovery API used for capture. Untitled windows use the application
name; a missing Quartz window title no longer removes them from the picker.
Minimized/off-screen windows and Koma's own windows are excluded. Source discovery
does not capture screenshots or traverse each application's Accessibility tree.
After an update, fully quit and relaunch the rebuilt GUI executable; the picker
and native adapter run in the GUI process. Open the picker again to refresh sources.

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
can prevent `type`. Named key chords are sent as keycodes and are not blocked
by the active group, Caps Lock, or a sticky modifier. Pointer moves, clicks,
and scrolls are not blocked either. An unknown key name stays shared and is
not reported as a desktop change. Mapping delivery
uses short bounded delays and still needs validation against actual applications.
Release physical keys/buttons before letting the agent act.

**Windows capture:** one Graphics Capture session per observation. The session
keeps two buffers and, for up to 100 ms after the first frame, replaces it with
any newer frame. A desktop that does not change again returns the first frame.
A second session is not opened for that poll.

**macOS capture:** after the shared post-batch wait, one ScreenCaptureKit
screenshot. There is no second shot and no frame poll.

**Wayland:** choose a screen or application with **Choose … in system dialog**
in the corresponding picker section. The compositor must expose that source type
through the
[ScreenCast portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html).
Source-type support depends on the compositor. The user owns source selection; programmatic
listing/switching is unavailable. Consent has a 120-second timeout. A one-frame
GStreamer consumer uses the portal's restricted PipeWire descriptor for each
observation/preview. The portal session stays open across Pause/Take back control;
Stop, disconnect, revocation and session changes close it. This implementation does
not implement RemoteDesktop input, fresh region capture, or desktop accessibility
matching. New portal IDs retain whether the selected source is a screen or application.
It does not substitute XWayland input or restore an old grant silently.

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

Picker layout regression: with the GUI dev server and a headless Chrome instance
already running with a remote debugging port, run:

```sh
KOMA_GUI_URL=http://127.0.0.1:5173 KOMA_BROWSER_CDP_URL=http://127.0.0.1:9222 node scripts/check_computer_picker.mjs
```

This uses fixture sources in the detached-viewer web route and never captures or
controls the desktop. Browser checks passed for category/row visibility at
560×315, 420×236, 240×135 and 320×120, application selection and focus restoration,
deferred refresh while busy, and application portal request routing. GUI TypeScript
checks and the production build passed (existing bundle-size warnings remain).
This is not native macOS/Windows/Linux capture validation; the macOS discovery
change still requires an SDK build and device check.

Current desktop-sharing verification is recorded separately from native runtime
validation. No desktop capture/input or portal smoke tests were run for this change.
The macOS and Windows bridges require their native SDK builds; Linux compilation
does not validate those branches. Release packaging scripts include the scaling
plugin, but no installer/AppImage has been built for this change.

Desktop-sharing baseline validation on the Linux host:

- `cargo check --workspace --offline` passed.
- `cargo test -p agent --bin koma --offline computer -- --skip native`: 22 passed.
- GUI and headless workspace all-target Clippy passed with `-D warnings`.
- GUI `tsc --noEmit` and Vite production build passed (existing chunk-size warnings).
- Four React server-render assertions passed for compact headers, error details and
  theme classes. The standalone portable C++ sizing helper compiled with
  `-std=c++17 -Wall -Wextra -Werror` and passed its resolution assertions.
- Packaging Python syntax and `git diff --check` passed.

Key-chord follow-up: the computer subset now passes 25 tests, including both
reported Spotlight spellings, canonical keys reaching adapter preflight, unsupported
key rejection preserving controller state, and preflight failures reporting zero
uncertain input. GUI/headless all-target Clippy with `-D warnings` also passed for
this follow-up. macOS/Windows SDK compilation and native shortcut tests were not run.

Assist-mode follow-up: 27 computer tests passed, including read-only application
selection without focus, refusal of every input kind, application-to-screen
transition requiring a new observation, and recovery that retains ownership. GUI
TypeScript/build and GUI/headless all-target Clippy with `-D warnings` passed.
Static React rendering checks passed for the two picker sections, portal choices,
source classification and compact recovery details. Native SDK/device checks remain
unexecuted, including the updated opt-in X11 fixture.

These are compilation/automated checks, not native SDK or device evidence. Regression
coverage includes 4K/8K/portrait/ultrawide budgets, negative-origin and Retina-point
mapping, fresh close-up mapping, app switching, stale coordinates, partial outcomes,
cancellation, ownership and preview correlation, and preserved screenshot attachments
when enrichment fails. Full native validation remains for the device walkthrough.

1. On macOS, Windows, or X11, share an application window and confirm it is not
   labelled view-only. A click brings that window forward and lands inside its
   screenshot. On Wayland, the share stays view-only and the model selects a screen.
   Share a screen with `docs/testing/computer-fixture.html` open. Verify the model
   and chat card receive the same bounded PNG; the preview can show newer frames.
2. Put another application/dialog over the fixture. Observe the composed desktop,
   click/switch apps, observe again, then type `Koma42 — café 世界 🌍`. Ordinary overlap
   must not end sharing. Try click → type and key → type batches; confirm one final frame and correct focus.
3. Test 4K, 8K, portrait and mixed-DPI displays, including negative desktop origins.
   Verify output dimensions and click accuracy. Request a small `region`, read its
   finer text and click within it. An empty observe must restore the full display.
   `crop` must create no desktop capture; neither operation may reuse stale focus.
4. Show the in-app and detached previews over the intended target. Verify they hide
   before model capture/input and reappear without taking focus. Stop sharing must
   close them. Check CPU/memory and capture latency on an 8K display.
5. Change foreground app or display geometry after observation and before input. Input must not use
   stale coordinates; safe pre-input changes should ask for a fresh observation
   while keeping sharing active. Take back control mid-sequence: future input stops,
   keys release, live preview continues. Give control requires a fresh observation.
6. Switch sessions or compete from a second GUI. Reactivation must remain explicit.
   If the owning window drops while the agent is still iterating, sharing stays on:
   the same window reattaches, or a new one opens on that session, and the turn
   continues. An idle share still stops. Verify native control also works in Plan
   mode, with no TUI/headless/subagent tools.
7. With the preview closed, idle/intermediate `observe=false` actions must create no
   timed captures. Open preview and check bounded live frames without model artifacts.
   Inspect errors in chat: header short, explanation only inside expandable details.
8. Revoke permissions and check stopped-control recovery. Wayland must remain
   observation-only, survive pause/resume without a new grant, and close its portal
   on Stop. Test missing OCR/scaling plugins and report their capability limitations.

Record OS, SDK, architecture, monitor dimensions/scales, permission state and exact
errors separately from unit-test and compile results. Native X11 fixture tests remain
opt-in. The fixture checks that application input is refused before switching to a
screen for click/type/observe; this does not substitute for mixed-DPI validation.
