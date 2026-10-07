#!/bin/bash
# run-gpu.sh <name> <probe-script-file>
# E8b review 1 (from E8 review 2's driver): a release daemon in the
# session session.sh lays out, the release GPU's headless popup probe
# (PMACS_GPU_PROBE_ACTION=popup: every `key:` step through
# App::apply_keyboard, the production key path) driving the script,
# then the daemon stopped by pid. Each `report:<label>` writes the band
# as `<label>.status`. Report: ~/build/e8b-review1/witness/<name>/report.txt
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
R=/home/jeans/build/cargo-target/release
eval "$("$HERE/session.sh" "$1")" || exit 2
SCRIPT=$(tr '\n' ' ' < "$2")
start=$(date +%s)
PMACS_GPU_PROBE_ACTION=popup PMACS_GPU_PROBE_TYPE_TEXT="$SCRIPT" PMACS_GPU_PROBE_DEADLINE_MS=${DEADLINE_MS:-300000} \
  "$R/pmacs-gpu" --headless-probe "$SOCK" "$W/report.txt" > "$W/probe.log" 2>&1
echo "probe_exit=$? elapsed=$(( $(date +%s) - start ))s" >> "$W/report.txt"
kill -TERM "$DPID" 2>/dev/null; for _ in $(seq 100); do kill -0 "$DPID" 2>/dev/null || break; sleep 0.1; done
kill -0 "$DPID" 2>/dev/null && { echo "daemon still alive, KILL" >> "$W/report.txt"; kill -KILL "$DPID"; }
echo DONE >> "$W/report.txt"
echo "$W/report.txt"
