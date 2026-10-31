# Terminal memory diagnostics

Set `KOMA_TERM_MEMLOG=1` before starting `koma` to append one JSON record per
second to `koma-term-memory-<pid>.jsonl` in the operating system's temporary
directory. The file contains process memory counters, counts of full snapshots,
deltas and frame bytes received from the daemon and, while chat is mounted,
loaded and mounted message counts, mounted turn counts, an estimate of loaded
message content bytes (content, reasoning, tool calls and tool results), and
whether a response is streaming. It does not contain message text. Bun's
counters are not the same as the operating system's total physical footprint.
With the variable unset, none of this is measured.

## Daemon updates

The terminal opts in to `SessionProgress` deltas when the daemon's `Hello`
lists the `session_progress` capability. While a turn runs, the daemon then
sends only the messages appended since the client's last baseline, plus the
streaming buffers, token counts, cost and context usage. History rewrites
(compaction, rewind, edits to earlier messages) still arrive as a full
snapshot. If a delta does not line up with the folded history, the terminal
requests a resync and ignores deltas until the next snapshot. Clients that do
not opt in keep receiving full snapshots.

## Render window

The transcript follows the newest 30 turns. Scrolling into older history mounts
a moving window of at most 90 turns; crossing either edge pages in that direction
and unmounts the far end. The window is also capped at 2 MiB of message content
(`KOMA_TERM_WINDOW_BYTES`, in bytes); the first turn always mounts, however
large. OpenTUI's native anchor keeps a retained turn in place during the layout
change. Click `Latest` to return to the live tail. Conversation data stays in
the client snapshot so older turns can be revisited.

The window moves in the same render as the history it follows. OpenTUI keeps
some native memory for every destroyed renderable, so a commit that mounts a
stale window and then trims it grows the process on each update.

## Memory pressure

On Unix, a five-second RSS check enters reduced mode above 768 MiB and leaves it
below 512 MiB. Reduced mode shrinks the window to 30 turns and half the byte
budget around the current reading position, then requests synchronous Bun GC
after layout commits, at most once per 30 seconds. If GC frees less than 32 MiB,
later cleanups skip it and the footer says the history window was reduced. Above
2 GiB the footer suggests restarting the terminal, which keeps the session; the
client is never killed or restarted. Set the thresholds with
`KOMA_TERM_MEM=high,low,warn` in MiB. `SIGUSR2` requests the same cleanup for
diagnostics. Windows receives the moving window and allocation fixes but has no
pressure signal handler.

## Regression tests

```
cd src-term
.tools/bun test src/history-window.test.tsx
```

The soak test appends 2,000 turns through `SessionProgress` deltas in ten cycles,
pages back four times per cycle while checking the anchor and mounted-turn
counts, returns to the tail, then rewinds history with a full snapshot. RSS must
grow by less than 4 MiB per cycle over the second half. The second test loads
3,000 synthetic turns, scrolls in both directions, exercises cleanup on Unix,
and checks RSS growth against a 160 MiB allowance after the initial paint. These
are synthetic regressions, not a guarantee for every conversation. A single
unusually large turn and the full snapshot can still take substantial memory.

The terminal launcher no longer enforces `KOMA_TERM_MAX_MB`. Existing settings
for that variable have no effect. The operating system can still terminate a
process if the machine runs out of memory.
