#!/usr/bin/env bash
# Launch a real desktop binary, render a frame, and round-trip a document.
# Run in a disposable display, e.g. xvfb-run -a dbus-run-session -- bash
# packaging/linux/smoke-test.sh /usr/bin/photocraft /usr/bin/photocraft-cli.
# Requires bash, jq, and an X11/Wayland display with Mesa or a working GPU driver.
set -euo pipefail

APP="$(realpath "${1:-target/debug/photocraft}")"
CLI="$(realpath "${2:-$(dirname "$APP")/photocraft-cli}")"
PORT="${PHOTOCRAFT_SMOKE_PORT:-48787}"
TMP="$(mktemp -d)"
PID=
cleanup() {
  if [ -n "$PID" ]; then
    kill "$PID" 2>/dev/null || true
    wait "$PID" 2>/dev/null || true
  fi
  if [ -n "${PHOTOCRAFT_SMOKE_ARTIFACTS:-}" ]; then
    mkdir -p "$PHOTOCRAFT_SMOKE_ARTIFACTS"
    for artifact in app.log smoke.png ui.json cli.json; do
      if [ -f "$TMP/$artifact" ]; then cp "$TMP/$artifact" "$PHOTOCRAFT_SMOKE_ARTIFACTS/"; fi
    done
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT
fail() { echo "FAIL: $*" >&2; cat "$TMP/app.log" >&2; exit 1; }

mkdir -p "$TMP/config" "$TMP/runtime"
chmod 700 "$TMP/runtime"
PHOTOCRAFT_CONFIG_DIR="$TMP/config" XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$TMP/runtime}" \
  "$APP" --control "$PORT" --control-token-file "$TMP/control.token" \
  --automation-read-root "$TMP" --automation-write-root "$TMP" >"$TMP/app.log" 2>&1 &
PID=$!

connected=0
for ((attempt = 0; attempt < 120; attempt++)); do
  kill -0 "$PID" 2>/dev/null || fail "desktop exited before opening a window"
  if [ -s "$TMP/control.token" ] && { exec 3<>"/dev/tcp/127.0.0.1/$PORT"; } 2>/dev/null; then
    connected=1
    break
  fi
  sleep 1
done
[ "$connected" = 1 ] || fail "desktop control server did not become ready"

request() {
  printf '%s\n' "$1" >&3
  IFS= read -r -t 60 reply <&3 || fail "control reply timed out or connection closed"
  jq -e '.ok == true' <<<"$reply" >/dev/null || fail "control request failed: $reply"
}
token="$(tr -d '\r\n' <"$TMP/control.token")"
request "{\"id\":1,\"method\":\"auth\",\"params\":{\"token\":\"$token\"}}"
request '{"id":2,"method":"engine.execute","params":{"command":"file.new","params":{"width":64,"height":48,"background":"white"}}}'
request '{"id":3,"method":"app.save","params":{"path":"smoke.pcraft"}}'
request '{"id":4,"method":"app.open","params":{"path":"smoke.pcraft"}}'
request '{"id":5,"method":"ui.inspect"}'
# Mesa's software adapter intentionally opens the CPU recovery notice. Acknowledge
# only that expected reason, using the button in the centred default dialog, so
# the screenshot below shows the document. Unexpected GPU failures remain errors.
if jq -e '.result.gpuFallbackNotice == "No compatible hardware graphics adapter; using software graphics."' <<<"$reply" >/dev/null; then
  request "$(jq -c '{id:"keep-cpu",method:"ui.click",params:{x:(.result.window.width / 2 - 130),y:(.result.window.height / 2 + 84)}}' <<<"$reply")"
  for ((attempt = 0; attempt < 20; attempt++)); do
    request '{"id":"gpu-notice","method":"ui.inspect"}'
    if jq -e '.result.gpuFallbackNotice == null' <<<"$reply" >/dev/null; then break; fi
  done
fi
jq -e '.result.gpuFallbackNotice == null' <<<"$reply" >/dev/null || fail "GPU recovery notice obscures the document"
printf '%s\n' "$reply" >"$TMP/ui.json"
jq -e '.result.document.width == 64 and .result.document.height == 48' "$TMP/ui.json" >/dev/null
request '{"id":6,"method":"ui.screenshot","params":{"path":"smoke.png"}}'
jq -e '.result.width > 0 and .result.height > 0' <<<"$reply" >/dev/null
[ -s "$TMP/smoke.png" ] || fail "rendered screenshot is missing"
request '{"id":7,"method":"app.quit"}'
exec 3>&-

for ((attempt = 0; attempt < 30; attempt++)); do
  if ! kill -0 "$PID" 2>/dev/null; then break; fi
  sleep 1
done
kill -0 "$PID" 2>/dev/null && fail "desktop did not quit"
wait "$PID" || fail "desktop exited unsuccessfully"
PID=
"$CLI" info "$TMP/smoke.pcraft" --compact >"$TMP/cli.json"
jq -e '.width == 64 and .height == 48' "$TMP/cli.json" >/dev/null
echo "Linux desktop rendering, save/reopen, clean shutdown, and headless CLI ok"
