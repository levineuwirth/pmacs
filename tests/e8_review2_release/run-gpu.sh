#!/bin/bash
# run-gpu.sh <name> <file> <probe-script> [extra-init-lua]
# E8 review 2 (from fix round 1 and review 1): a release daemon visiting <file> with
# real servers and isolated XDG roots, the release GPU's popup probe
# (PMACS_GPU_PROBE_ACTION=popup: App::apply_keyboard, apply_wheel,
# apply_left_button) driving <probe-script>, then the daemon stopped by
# pid. Report: ~/build/e8-review2/witness/<name>/report.txt
set -u
R=/home/jeans/build/e8-review2/rel-target/release
W=/home/jeans/build/e8-review2/witness/$1
FILE=$2
SCRIPT=$3
EXTRA=${4:-}
if [ -e "$W" ]; then echo "exists: $W" >&2; exit 2; fi
mkdir -p "$W/run" "$W/cfg/pmacs" "$W/data" "$W/state" "$W/cache" "$W/tmp"; chmod 700 "$W/run"
cat > "$W/cfg/pmacs/init.lua" <<EOF
pmacs.buffer.find_or_open([[$FILE]])
for _, b in ipairs(pmacs.buffer.list()) do
  if b:name() == '*scratch*' then pcall(pmacs.buffer.kill, b) end
end
$EXTRA
EOF
export XDG_CONFIG_HOME=$W/cfg XDG_DATA_HOME=$W/data XDG_STATE_HOME=$W/state XDG_CACHE_HOME=$W/cache PMACS_STATE_HOME=$W/state TMPDIR=$W/tmp
export CARGO_TARGET_DIR=/home/jeans/build/e8-review2/ra-target
export PMACS_INSTANCE_SEMANTIC_RENDER=1 PMACS_INSTANCE_MULTI_FRONTEND=1
nice -n 10 "$R/pmacs" --daemon --socket "$W/run/w.sock" > "$W/daemon.log" 2>&1 &
DPID=$!
for _ in $(seq 100); do [ -S "$W/run/w.sock" ] && break; sleep 0.1; done
start=$(date +%s)
PMACS_GPU_PROBE_ACTION=popup PMACS_GPU_PROBE_TYPE_TEXT="$SCRIPT" PMACS_GPU_PROBE_DEADLINE_MS=${DEADLINE_MS:-300000} \
  "$R/pmacs-gpu" --headless-probe "$W/run/w.sock" "$W/report.txt" > "$W/probe.log" 2>&1
echo "probe_exit=$? elapsed=$(( $(date +%s) - start ))s" >> "$W/report.txt"
kill -TERM "$DPID" 2>/dev/null; for _ in $(seq 100); do kill -0 "$DPID" 2>/dev/null || break; sleep 0.1; done
kill -0 "$DPID" 2>/dev/null && { echo "daemon still alive, KILL" >> "$W/report.txt"; kill -KILL "$DPID"; }
echo DONE >> "$W/report.txt"
