#!/bin/bash
# session.sh <name>
# E8b review 1: lay out one witness session for the release pair and
# start its daemon. Prints the witness directory, the project, the
# socket and the daemon's pid as shell assignments (eval them).
#
# The project lives under ~/build/e8b-review1/witness/<name>/proj, a
# root that spells `b` (build) and `nts` (jeans, witness), as the gate's
# temporary root and E8 review 1's did. It holds a.rs, b.rs, notes.txt,
# dired.lua, editor.rs and src/lsp.rs; `*scratch*` is kept. The init
# shows a.rs and binds `C-c z` to `review.where`, which writes the
# window's buffer to the band as `WHERE <name>`: an oracle for the
# window that does not read E8b.2's own message.
set -u
R=/home/jeans/build/cargo-target/release
W=/home/jeans/build/e8b-review1/witness/$1
P=$W/proj
if [ -e "$W" ]; then echo "exists: $W" >&2; exit 2; fi
mkdir -p "$W/run" "$W/cfg/pmacs" "$W/data" "$W/state" "$W/cache" "$W/tmp" "$P/src"
chmod 700 "$W/run"
for f in a.rs b.rs notes.txt dired.lua editor.rs src/lsp.rs; do echo "// $f" > "$P/$f"; done
cat > "$W/cfg/pmacs/init.lua" <<EOF
pmacs.lsp.config = {}
local proj = [[$P]]
for _, f in ipairs({ "a.rs", "b.rs", "notes.txt", "dired.lua", "editor.rs", "src/lsp.rs" }) do
  pmacs.buffer.find_or_open(proj .. "/" .. f)
end
for _, id in ipairs(pmacs.buffer.list()) do
  if pmacs.describe.buffer(id).name == proj .. "/a.rs" then pmacs.window.switch_buffer(id) end
end
pmacs.command.define {
  name = "review.where",
  description = "E8b review 1: say which buffer the window shows.",
  fn = function()
    pmacs.editor.set_status("WHERE " .. pmacs.describe.buffer(pmacs.window.buffer()).name)
  end,
}
pmacs.keymap.bind { scope = "global", sequence = "C-c z", command = "review.where" }
EOF
export XDG_CONFIG_HOME=$W/cfg XDG_DATA_HOME=$W/data XDG_STATE_HOME=$W/state XDG_CACHE_HOME=$W/cache PMACS_STATE_HOME=$W/state TMPDIR=$W/tmp
export PMACS_INSTANCE_SEMANTIC_RENDER=1 PMACS_INSTANCE_MULTI_FRONTEND=1
nice -n 10 "$R/pmacs" --daemon --socket "$W/run/w.sock" > "$W/daemon.log" 2>&1 < /dev/null &
DPID=$!
for _ in $(seq 100); do [ -S "$W/run/w.sock" ] && break; sleep 0.1; done
echo "W=$W P=$P SOCK=$W/run/w.sock DPID=$DPID"
