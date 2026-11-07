# Cyber mode

A coding agent with a security lane bolted on the side is the wrong shape: the
capability leaks, the boundary is wherever the prompt last said it was, and a
"confirmed" finding turns out to be a tool exit code. This document is the
contract the code enforces, and where each piece of it lives.

## The three lanes

Capability is a matrix, enforced at dispatch by `app/sec/lane.rs`. A sentence in a
prompt is advice; the matrix is checked before a tool is admitted.

| capability          | research | red | blue |
|---------------------|----------|-----|------|
| read source/corpus  | yes      | yes | yes  |
| reach the network   | no       | yes | yes  |
| start a listener    | no       | yes | no   |
| propose a patch     | yes      | no  | yes  |
| apply a patch       | no       | no  | no   |

Research reads without touching a target. Only red listens. Blue proposes but
never applies, so a blue fix still goes through the user's own diff review. An
unknown `sec_` tool is classified `network` — the strict side, because assuming a
new tool is free lets the research lane open a socket.

The matrix is pinned in one test, so loosening it shows up as a diff.

## The boundary

The guest is on an `--internal` Docker network. That is the whole guarantee: no
gateway means nothing outside is reachable, whatever the guest believes. A proxy
container is the only peer and the only route. Proxy environment variables are how
a client *learns* the route, never the boundary itself — a test proves that a
direct attempt at a correct IP still fails.

The proxy is deny-by-default and CONNECT-only, re-checks every connection against
a rules file it reloads on change, and refuses rather than tunnels. Unsupported
protocols fail closed.

Runtime flags, so nothing depends on a setup step being remembered: non-root,
`--cap-drop ALL`, `no-new-privileges`, read-only root with `/tmp` and `/artifacts`
the only writable paths, no Docker socket.

A proxy refusal carries an `X-Koma-Egress` header, so a blocked call is
distinguishable from the target's own 403. Without it an egress stop was recorded
as if the target had answered.

## Scope

A versioned record. An edit bumps `version` and a call stamped with an older one is
refused, never silently upgraded — broadening a scope mid-flight cannot turn an
admitted call into an unchecked one. Absent scope means no network at all.

Exclusions are checked before targets, so an exclusion always wins. Host
normalisation happens once at parse time. Redirect hops are re-admitted one by one
and the first refusal names the hop that broke.

## Evidence and findings

A verdict alone is not proof. `confirmed` requires a handle that still resolves to
retained bytes, and a confirmation whose evidence vanished is **demoted** to
`unproven` in the report rather than dropped — losing it silently would look like
it was never claimed.

`untested` is a separate bucket from `unproven` throughout, because collapsing them
is how a gap starts reading as a negative result.

Evidence keeps the raw bytes beside the redacted copy. Redaction that drops the
original loses the thing worth keeping.

## The board

Shared by the lanes, transactional. Every read-modify-write takes an `flock`; atomic
file replacement alone loses updates under concurrency, which the test proves with
four real processes. The lock is kernel-released on death, so a crash does not wedge
the board.

Claims are leased, so a crash is recoverable without a reaper. The
duplicate-execution check runs at dispatch, *before* the tool, so two lanes cannot
fire at one route.

## Supervision

Hypothesis cap, turn budget, rate and total caps. At the soft turn limit the run is
warned once; at the hard limit it is paused rather than being allowed to finish a
thought that already overran. Resuming grants turns instead of resetting the count,
so a pause is not a free extension.

## Degrading

One missing dependency costs one tool, not the daemon. Registration is isolated per
module; a tool whose dependency is absent is advertised as `unavailable` with an
install hint and returns a typed `unavailable` if called. Nothing auto-installs, so
the frame reader is never blocked by pip.

## Where the browser sits

`sec_browser` runs on the HOST, not in the guest — playwright pulls ~300 MB of
driver that would load on every run even for pure-HTTP targets. `exec: browser` is a
distinct class so the placement is visible. The browser is pointed at the run's
egress proxy, which is what keeps a host-side navigation inside the scope.

## Verifying

    cd src-security && python3 -m unittest discover -t .

Skips are real "unverified", with a stated reason. The boundary suite needs Docker
and a built image; the browser suite needs playwright.

## Known limits

- macOS/colima will not bind-mount a newly created directory under `/tmp`; boundary
  state lives under `$HOME` for that reason.
- The daemon's package is shipped into the image as a copy. `test_hardening`
  compares the two and fails on drift, so re-copy before building.
- The image carries no per-tool CLIs; those show as `unavailable` with a hint until
  installed.
