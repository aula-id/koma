# Persistent SSH coding service

On Unix and Windows remote hosts, `coding-worker` is a transport proxy to a per-user
`coding-daemon`. The daemon owns tasks, DAP sessions, test results, language
processes and filesystem watchers. Reopening Tasks/Debug/Tests after reconnect
queries the same process registries; no command is replayed to reconstruct a job.
A lost response still reports an unknown outcome, so inspect running jobs before
manually repeating a start or write.

The Unix socket is `~/.koma/run/coding-service/coding-v3.sock`, inside an owner-checked
0700 directory, with a 0600 socket and a no-follow lock file. A held advisory lock
serializes startup and stale-socket cleanup. No TCP listener is exposed. The
proxy spawns a detached daemon using the same remote executable. Up to eight
clients are accepted; output backpressure disconnects a slow client rather than
silently dropping language replies. Frames retain the coding protocol's 48 MiB
limit. A daemon exits after thirty minutes with no clients or active jobs.

Windows uses Koma's named-pipe transport at `\\.\pipe\koma-coding-v3-<user-path-hash>`.
Each instance grants access to its owner, SYSTEM and administrators; the first
instance owns the name. The daemon is detached from the SSH process and requests
job breakaway. A Windows SSH host must permit that breakaway; failure is reported
instead of silently providing connection-lifetime tasks. Tasks and PTYs still
use their own kill-on-close jobs for cancellation.

GUI shutdown closes its SSH proxy but leaves remote jobs available for adoption.
Use Stop/Cancel in the corresponding panel to terminate a job. Local coding still
belongs to the local GUI process.
Daemon state is memory-resident, so a host reboot or daemon crash does not preserve
processes or results. Unsaved-document backups remain a separate durable facility.

On connection, the proxy compares its executable digest with the daemon's
startup digest. A mismatch requests draining. With no other clients or active
jobs, the proxy disconnects and reconnects to a replacement daemon automatically.
Otherwise it keeps using the existing service until all jobs finish and clients
disconnect. No running job is migrated or stopped for an upgrade, and the binary
must already have been updated on the remote host. Connected clients can defer
replacement indefinitely. Failed negotiation does not replay user operations.
After a successful transport connection, mounted editor buffers are reopened to
the language service, including when a replacement daemon has no snapshots.

The v2-to-v3 protocol change uses a separate address. Finish/stop v2 jobs using the
old client before upgrading; v3 cannot adopt their in-memory records. Automatic
digest-based replacement applies between builds of this v3 protocol.

Language managers are keyed by GUI-window ID plus host/root; diagnostics and
server-initiated edits return only to that window. Normal window close releases
its managers. Abrupt client failure can retain a manager until service shutdown;
there is a 128-manager limit. This isolates language state but does not merge
concurrent disk edits.

Native acceptance: start a long task/test/debug session over SSH, disconnect the
transport, reconnect, inspect output and stop it. Repeat with two roots and chats,
response loss immediately after Start, simultaneous reconnects, stale sockets,
slow clients, and no active jobs. Repeat on Windows with permitted/denied SSH job
breakaway, and with a newer remote executable while jobs are running and idle.
The agent compiled the Unix GUI and Windows non-GUI paths but did not execute
these scenarios.
