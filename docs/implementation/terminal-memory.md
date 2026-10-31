# Terminal memory diagnostics

Set `KOMA_TERM_MEMLOG=1` before starting `koma` to write one JSON record per
second to `koma-term-memory-<pid>.jsonl` in the operating system's temporary
directory. The file contains process memory counters and, while chat is
mounted, loaded and mounted message counts, mounted turn counts, an estimate
of loaded message content bytes, and whether a response is streaming. It does
not contain message text. The content estimate excludes tool metadata and other
snapshot data. Bun's counters are not the same as the operating system's total
physical footprint.

The transcript follows the newest 30 turns. Scrolling into older history mounts
a moving window of at most 90 turns; crossing either edge pages in that direction
and unmounts the far end. OpenTUI's native anchor keeps a retained turn in place
during the layout change. Click `Latest` to return to the live tail. Conversation
data stays in the client snapshot so older turns can be revisited without a
daemon protocol change.

On Unix, a five-second RSS check requests cleanup above 512 MiB. Cleanup shrinks
the render window to 30 turns around the current reading position and requests
synchronous Bun GC after layout commits. Requests are limited to one per 30
seconds. Repeated cleanup displays a persistent hint; it never kills or restarts
the client. `SIGUSR2` requests the same cleanup for diagnostics. Windows receives
the moving window and allocation fixes but has no pressure signal handler.

Run the isolated native-render regression with:

```
cd src-term
.tools/bun test src/history-window.test.tsx
```

It loads 3,000 synthetic turns, scrolls in both directions, exercises cleanup on
Unix, and checks mounted-turn counts and RSS growth against a 160 MiB allowance
after the initial paint. This is a synthetic regression, not a guarantee for every
conversation. A single unusually large turn and the full snapshot can still take
substantial memory; this change bounds mounted turns rather than all allocations.

The terminal launcher no longer enforces `KOMA_TERM_MAX_MB`. Existing settings
for that variable have no effect. The operating system can still terminate a
process if the machine runs out of memory.
