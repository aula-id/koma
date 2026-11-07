#!/bin/sh
# Guest entrypoint. Execs the command so signals reach it directly and there is no
# shell between the runtime and the process being cancelled.
#
# No setup runs here on purpose: anything that must be true about the guest
# (read-only root, non-root, no capabilities) is enforced by the runtime, not by
# this script. A setup step here would be a thing that can be forgotten.
set -eu

if [ "$#" -eq 0 ]; then
    set -- sleep infinity
fi

exec "$@"
