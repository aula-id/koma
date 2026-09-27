# Persistent SSH coding service

On Unix remote hosts, `coding-worker` is now a transport proxy to a per-user
`coding-daemon`. The daemon owns tasks, DAP sessions, test results, language
processes and filesystem watchers. Reopening Tasks/Debug/Tests after reconnect
queries the same process registries; no command is replayed to reconstruct a job.
A lost response still reports an unknown outcome, so inspect running jobs before
manually repeating a start or write.

The socket is `~/.koma/run/coding-service/coding-v2.sock`, inside an owner-checked
0700 directory, with a 0600 socket and a no-follow lock file. A held advisory lock
serializes startup and stale-socket cleanup. No TCP listener is exposed. The
proxy spawns a detached daemon using the same remote executable. Up to eight
clients are accepted; output backpressure disconnects a slow client rather than
silently dropping language replies. Frames retain the coding protocol's 48 MiB
limit. A daemon exits after thirty minutes with no clients or active jobs.

GUI shutdown closes its SSH proxy but leaves remote jobs available for adoption.
Use Stop/Cancel in the corresponding panel to terminate a job. Local coding still
belongs to the local GUI process. Windows remote hosts retain the original
stdio worker lifecycle; persistent adoption there requires a named-pipe service.
Daemon state is memory-resident, so a host reboot or daemon crash does not preserve
processes or results. Unsaved-document backups remain a separate durable facility.
Remote binary upgrades require the same protocol generation; automatic live
migration between daemon versions is not implemented.

Native acceptance: start a long task/test/debug session over SSH, disconnect the
transport, reconnect, inspect output and stop it. Repeat with two roots and chats,
response loss immediately after Start, simultaneous reconnects, stale sockets,
slow clients, and no active jobs. The agent compiled this code but did not execute
these scenarios.
