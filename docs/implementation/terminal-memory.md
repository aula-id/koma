# Terminal memory diagnostics

Set `KOMA_TERM_MEMLOG=1` before starting `koma` to write one JSON record per
second to `koma-term-memory-<pid>.jsonl` in the operating system's temporary
directory. The file contains process memory counters and, while chat is
mounted, loaded and mounted message counts, mounted turn counts, an estimate
of loaded message content bytes, and whether a response is streaming. It does
not contain message text. The content estimate excludes tool metadata and other
snapshot data. Bun's counters are not the same as the operating system's total
physical footprint.

The terminal launcher no longer enforces `KOMA_TERM_MAX_MB`. Existing settings
for that variable have no effect. The operating system can still terminate a
process if the machine runs out of memory.
