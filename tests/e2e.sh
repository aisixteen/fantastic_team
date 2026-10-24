#!/bin/sh
# sh tests/e2e.sh BIN — model-free e2e on one built fantastic_team binary (#46 full run).
# Checks: a headless boot stays up with a clean log; a --port boot serves the canvas
# page; the secrets guard runs over this run's output. The scripted fake-model master
# with 2 workers against a local bare git remote is a TODO: it needs the fake-model
# harness (#38, #8), which is not on main yet.
# Exit 0: all checks pass. 1: a check failed. 2: no binary.
set -u
BIN=${1:?usage: sh tests/e2e.sh PATH_TO_fantastic_team}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
case "$BIN" in
  /*) ;;
  *) BIN="$(pwd)/$BIN" ;;
esac
if [ ! -f "$BIN" ]; then
  echo "e2e: no binary at $BIN" >&2
  exit 2
fi
PORT=${E2E_PORT:-18931}
OUT=$(mktemp -d "${TMPDIR:-/tmp}/fantastic_e2e.XXXXXX")
git -C "$OUT" init -q
PASS=0
FAIL=0
PID=
ok() { PASS=$((PASS + 1)); echo "ok   $1"; }
bad() { FAIL=$((FAIL + 1)); echo "FAIL $1${2:+ — $2}"; }
stop() {
  if [ -n "$PID" ]; then
    kill "$PID" 2>/dev/null
    wait "$PID" 2>/dev/null
    PID=
  fi
}
trap stop EXIT

# 1. Headless boot (no flags): no port, no terminal UI. It runs until stopped, so it
#    must still be alive after a while, with no panic or error in its log.
H="$OUT/headless"
mkdir -p "$H"
(cd "$H" && exec "$BIN" >"$OUT/headless.log" 2>&1) &
PID=$!
sleep 8
if kill -0 "$PID" 2>/dev/null; then
  ok "headless boot stays up"
else
  bad "headless boot exited early" "see $OUT/headless.log"
fi
stop
if grep -qiE 'panic|error' "$OUT/headless.log"; then
  bad "headless log is clean" "panic or error in $OUT/headless.log"
else
  ok "headless log is clean"
fi

# 2. --port boot: the binary prints the canvas page URL at start; it must answer 200 HTML.
P="$OUT/port"
mkdir -p "$P"
(cd "$P" && exec "$BIN" --port "$PORT" >"$OUT/port.log" 2>&1) &
PID=$!
URL=
i=0
while [ "$i" -lt 30 ]; do
  URL=$(sed -n 's#.*the canvas: \(http://[^ ]*\).*#\1#p' "$OUT/port.log" | head -1)
  [ -n "$URL" ] && break
  kill -0 "$PID" 2>/dev/null || break
  sleep 1
  i=$((i + 1))
done
if [ -z "$URL" ]; then
  bad "--port boot prints the canvas URL" "see $OUT/port.log"
else
  CODE=$(curl -s -m 10 -o "$OUT/page.html" -w '%{http_code}' "$URL")
  if [ "$CODE" = 200 ] && grep -qi '<html' "$OUT/page.html"; then
    ok "--port serves the canvas page (200, html)"
  else
    bad "--port serves the canvas page" "http $CODE from $URL"
  fi
fi
stop

# 3. Secrets guard over everything this run wrote (the output dir is a git work tree).
if sh "$ROOT/scripts/check-secrets.sh" "$OUT"; then
  ok "secrets guard clean over the run output"
else
  bad "secrets guard over the run output"
fi

# TODO(#38, #8): the scripted fake-model master with 2 workers, each doing one task
# (claim, a commit in a local bare git remote, done), then the claim files, leases
# and board counts, and the worker exec restriction. Not on main yet.

echo "e2e: $PASS passed, $FAIL failed (output in $OUT)"
[ "$FAIL" -eq 0 ]
