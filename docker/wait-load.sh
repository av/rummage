#!/usr/bin/env bash
# Waits while the machine is busy: until the 1-minute load average is at most MAX_LOAD
# (default 70% of the CPU threads), so a CI job doesn't pile a heavy build onto a loaded host.
set -euo pipefail
max=${MAX_LOAD:-$(($(nproc) * 7 / 10))}
while true; do
    load=$(cut -d' ' -f1 /proc/loadavg)
    if awk -v l="$load" -v m="$max" 'BEGIN { exit !(l <= m) }'; then
        echo "load $load (limit $max), starting"
        exit 0
    fi
    echo "load $load is above $max, waiting"
    sleep 30
done
