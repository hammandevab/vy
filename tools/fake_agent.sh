#!/bin/sh
# A stand-in for Claude Code in `vy-dev serve --agent-cmd`: no network, no
# credentials. It behaves like `claude -p ... --output-format stream-json`:
# the mission arrives on stdin, one JSON event per line goes to stdout, and
# it edits src/lib.rs in the working directory (the project), so the build
# vy-dev runs after the agent exits has something new to show.
#
#   vy-dev serve --agent-cmd tools/fake_agent.sh
#
# Records its arguments in fake-agent-args.txt and the mission in
# fake-agent-mission.txt. A mission containing SLOW sleeps 30 s in a child
# process (its pid in fake-agent-sleep.pid) before it finishes, so a stop
# can be tried; one containing FAIL ends with an error result and exit 3.
# The Windows twin is tools/fake_agent.ps1.
set -eu

printf '%s\n' "$@" > fake-agent-args.txt
cat > fake-agent-mission.txt

say() { printf '%s\n' "$1"; }

say '{"type":"system","subtype":"init","cwd":".","session_id":"fake","model":"fake-agent","tools":["Read","Edit","Bash"]}'
say '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Jag läser sidan och ändrar texten."}]}}'
say '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"src/lib.rs"}}]}}'
say '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"..."}]}}'

if grep -q SLOW fake-agent-mission.txt; then
  sleep 30 &
  echo $! > fake-agent-sleep.pid
  wait
fi

say '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Edit","input":{"file_path":"src/lib.rs","old_string":"Inga klick","new_string":"Agenten var har. Inga klick"}}]}}'
# Portable in-place edit: GNU and BSD sed disagree on -i.
sed 's/Inga klick/Agenten var har. Inga klick/' src/lib.rs > src/lib.rs.fake-agent
mv src/lib.rs.fake-agent src/lib.rs

if grep -q FAIL fake-agent-mission.txt; then
  say '{"type":"result","subtype":"error_during_execution","is_error":true,"result":"Något gick fel."}'
  exit 3
fi
say '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Klart: sidan hälsar från agenten."}]}}'
say '{"type":"result","subtype":"success","is_error":false,"result":"Sidan är ändrad.","total_cost_usd":0}'
