# Computer use: implementation and validation status

This is a partial implementation of the native computer-use plan. The daemon,
model-loop integration, GUI panel, deterministic tests, and Linux X11 adapter are
implemented. It is **not yet the planned cross-platform release**.

## What is implemented

Computer use starts disabled. In a local GUI session, open **Computer**, enable
control, refresh windows, and choose a visible window. GUI selection observes
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

Screenshots occur only on selection, explicit observation, or a requested final
action observation. `computer_act` defaults to `observe=true`. `observe=false`
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

The preview position and size are saved as GUI preferences. The optional native
viewer receives the same status/image and palette, with pause/stop/take-over
controls. It closes on session transition.

The same saved PNG is attached to the model and displayed by the preview. JSON
metadata under the session's `computer/` directory associates observations with
tool-call IDs; PNGs live under that session's `images/`. Image bytes travel only in
one-shot operation results, never recurring session snapshots. Context shaping
retains the latest actionable attachment. Desktop content is labelled as external
task data. Accessibility/OCR overlays do not change the PNG.

AT-SPI extraction matches the selected window by process identity, title and
bounds. It reads only labels/roles/states/bounds, skips password controls, traverses
at most 256 nodes and 12 levels, and uses a 750 ms traversal deadline with 100 ms
DBus method timeouts. Local English Tesseract OCR has a two-second deadline and
bounded output. Enrichment failure preserves a valid screenshot and component
status. OCR text never becomes a clickable accessibility target.

## Platform capability matrix

| Platform | Capture, listing, focus | Pointer / keyboard | Accessibility / OCR | Viewer |
| --- | --- | --- | --- | --- |
| Linux X11 | Implemented for local Unix-socket displays and EWMH windows with PID metadata | XTEST; typing limited to characters in the current keyboard map | AT-SPI, local Tesseract when installed/bundled | Draggable/resizable panel and detached viewer |
| Linux Wayland | Unavailable; portal adapter not implemented, including source-picker integration | Unavailable | No native observations | In-app panel with limitations |
| macOS | Unavailable; ScreenCaptureKit adapter not implemented | Unavailable | Accessibility / Vision adapter not implemented | In-app panel with limitations |
| Windows | Unavailable; Graphics Capture adapter not implemented | Unavailable | UI Automation adapter not implemented | In-app panel with limitations |

X11 capture/input reject an obstructed target, including obstruction by Koma's
preview. Koma's own windows and windows without process identity are excluded.
Coordinates use the X server's pixel space. A coordinate/focus check followed by
OS input still has a small desktop race; this is not an OS-level transactional
input API. XID/PID identity does not completely eliminate rare same-process XID
reuse. Native support should remain experimental pending broader validation.

## Packaging

The Linux Debian metadata requires Tesseract, English data, AT-SPI and XTEST.
AppImage metadata includes the Tesseract binary, language/config data, notices and
XTEST library. The release workflow installs those build-time inputs and runs
`scripts/check_computer_package.py` against the resulting artifact before upload.
That check extracts the artifact and loads English using its bundled executable
and libraries. These packaging paths follow the existing
[cargo-packager configuration](https://docs.crabnebula.dev/packager/configuration/).

Development/raw/custom binaries may lack Tesseract; this is reported as an OCR
limitation. OCR dependencies are confined to GUI builds. Windows OCR release
bundling is still outstanding; the WiX packaging system is unchanged.

## Remaining delivery work

- macOS ScreenCaptureKit, Accessibility, native input and Vision adapters, with
  native permissions and minimum-version handling.
- Windows Graphics Capture, UI Automation and native input adapters, plus bundled
  Tesseract runtime and English data in the existing WiX package.
- Wayland portal capture/input and user-mediated source selection.
- Broader native validation of the detached viewer on each desktop. The X11 viewer uses
  non-focusable window hints and pauses/falls back to the in-app panel if the
  compositor focuses it. Capture exclusion is unavailable on X11; obstruction
  checks remain active.
- General Unicode injection beyond the active X11 keyboard map.
- Broader native validation: mixed-DPI hardware, multiple monitors, compositor
  obstruction, permission revocation and all supported OS versions.

## Verification

Final Linux build checks passed with default GUI features and with
`--no-default-features`. Clippy passed in both configurations with `-D warnings`,
and the GUI TypeScript check passed. The default workspace suite passed **1,817
tests**, with the two native tests ignored in that run; each native test passed
when invoked explicitly. The headless computer regression subset passed **10
tests**. Commands used `--offline` for Cargo; tests needing local sockets and
application-data fixtures were run outside the sandbox.

An earlier full run exposed a lock-release race while other tests created child
processes. Controller and native-worker locks now explicitly unlock before closing,
and the regression retains a duplicate file handle while verifying takeover. The
final full-suite result above includes that fix.

Automated tests exercise an isolated desktop fixture and no desktop input unless
an ignored native test is explicitly selected. They cover capture count,
selection/click/type/observation, partial failure without replay, cleanup,
focus/geometry/closure rejection, cancellation, duplicate and late replies,
controller contention, generation changes after approval, model/preview byte
identity, missing enrichment, crop mapping, OCR provenance and delegated dispatch.

Commands:

```sh
cargo check --workspace
cargo check --workspace --no-default-features
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --no-default-features --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --no-default-features computer
cd src-webgui && npx tsc --noEmit
```

The X11 native test creates its own disposable fixture process, selects only that
window, injects one click and `Koma42`, checks the fixture's received input, and
captures a subsequent observation. It requires a local X11 desktop with an EWMH
window manager, XTEST, `cc` and X11 development headers:

```sh
cargo test --bin koma native_fixture_round_trip -- --ignored --nocapture
cargo test --bin koma native_viewer_preserves_focus_and_routes_controls -- --ignored --nocapture
```

For a GUI walkthrough, open `docs/testing/computer-fixture.html` locally in a
browser. Enable control and select that fixture. Verify the Apply counter and
message after approval; test scrolling as a separate sequence. Move the preview
over the target and verify refusal. Pause during approval, resize/close the target,
switch sessions, disconnect/reconnect, and compete from a second GUI. Confirm no
new PNGs appear while idle, and compare the preview artifact with the model's
attachment. Record native platform results separately from fake-adapter tests.

### Native evidence from this environment

On 2026-09-27, the opt-in `native_fixture_round_trip` test passed on the configured
X11 display (X.Org 1.20.14, 1280×720, XTEST present) after running outside the
filesystem/network sandbox. It verified window listing and focus, window capture,
a left click, typing `Koma42`, received fixture input, a changed final image, and
released injected keys/buttons. The separate native viewer smoke test also passed:
it opened a disposable target and detached WebView, preserved the target's focus,
and routed the viewer's Pause IPC without changing saved placement. WebKit emitted
a DRI3 hardware-acceleration warning on this display; the test still passed.
These tests did not exercise AT-SPI controls, Tesseract, multi-monitor DPI, a
Wayland session, macOS, or Windows. The viewer test checks native focus and IPC,
not a full model-driven GUI walkthrough or rendered-image visual comparison.
No release installer/AppImage was built or executed here; the package validation
gate is implementation, not packaging runtime evidence.
