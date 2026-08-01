#!/usr/bin/env bash
# WakeMATE Companion smoke driver.
#
# Launches the companion against a throwaway config + throwaway HOME, waits
# for the HTTP and HTTPS listeners, asserts the real API contract, then tears
# the server down.
#
#   ./smoke.sh                 # full smoke run, server stopped at the end
#   ./smoke.sh --keep-running  # leave it up and print how to talk to it
#   ./smoke.sh --input         # also enable + exercise input commands
#
# WHY THE FAKE HOME (this is the whole trick — do not remove it):
# The companion hydrates its pairing token from the macOS Keychain FIRST and
# only falls back to the config file. Run it against your real HOME and it
# stores the token in your login keychain, blanks `api_token` in the config,
# flips `token_storage` to "keyring" — and from then on the token you wrote
# into the config JSON is silently ignored. Reading it back out with the
# `security` CLI pops a "security wants to use your confidential information"
# password prompt, because `security` is not the binary that created the item.
#
# Pointing HOME at a scratch dir makes the keyring backend fail cleanly
# ("A default keychain could not be found"), so the app falls back to
# TokenStorage::File and uses the token below verbatim. That also relocates
# the TLS identity and the device registry — both of which normally live in
# ~/Library/Application Support/WakeMATE Companion and would otherwise
# survive `--config-path` isolation.
#
# Deliberately NOT exercised: power commands. `{"type":"system","action":
# "sleep"}` maps to `pmset sleepnow` on macOS and would suspend this machine.
set -uo pipefail

UNIT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
BIN="$UNIT_DIR/target/debug/wakemate-companion"
WORK="${TMPDIR:-/tmp}/wakemate-smoke.$$"
CONFIG="$WORK/config.json"
LOG="$WORK/server.log"
TOKEN="smoke-test-token"

HTTP_PORT=7787
TLS_PORT=7788
DISCOVERY_PORT=41244
KEEP_RUNNING=0
ENABLE_INPUT=0

while [ $# -gt 0 ]; do
  case "$1" in
    --keep-running) KEEP_RUNNING=1 ;;
    --input)        ENABLE_INPUT=1 ;;
    --port)         HTTP_PORT="$2"; TLS_PORT=$((HTTP_PORT + 1)); shift ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
  shift
done

PASS=0
FAIL=0
SERVER_PID=""

pass() { PASS=$((PASS + 1)); printf '  \033[32mPASS\033[0m %s\n' "$1"; }
fail() { FAIL=$((FAIL + 1)); printf '  \033[31mFAIL\033[0m %s\n' "$1"; printf '       got: %s\n' "$2"; }

# assert <label> <expected-substring> <actual>
assert() {
  case "$3" in
    *"$2"*) pass "$1" ;;
    *)      fail "$1" "$3" ;;
  esac
}

cleanup() {
  if [ -n "$SERVER_PID" ] && [ "$KEEP_RUNNING" -eq 0 ]; then
    kill "$SERVER_PID" 2>/dev/null
    wait "$SERVER_PID" 2>/dev/null
  fi
}
trap cleanup EXIT

if [ ! -x "$BIN" ]; then
  echo "error: $BIN not found — run 'cargo build' in $UNIT_DIR first" >&2
  exit 1
fi

mkdir -p "$WORK"

cat > "$CONFIG" <<EOF
{
  "bind_address": "127.0.0.1:$HTTP_PORT",
  "tls_port": $TLS_PORT,
  "allow_insecure_http": true,
  "discovery_port": $DISCOVERY_PORT,
  "discovery_message": "wakemate:discover",
  "api_token": "$TOKEN",
  "token_storage": "file",
  "device_name": "SmokeTestCompanion",
  "launch_on_startup": false,
  "allow_input_commands": $([ "$ENABLE_INPUT" -eq 1 ] && echo true || echo false),
  "allow_power_commands": false,
  "allow_remote_connections": false,
  "allow_discovery": false,
  "require_auth_for_info": true
}
EOF

echo "==> launching $BIN"
echo "    config: $CONFIG"
echo "    HOME:   $WORK  (isolates keychain + TLS identity + device registry)"
env HOME="$WORK" RUST_LOG=info "$BIN" --config-path "$CONFIG" > "$LOG" 2>&1 &
SERVER_PID=$!

BASE="http://127.0.0.1:$HTTP_PORT"
for _ in $(seq 1 50); do
  curl -sf -o /dev/null "$BASE/v1/health" && break
  kill -0 "$SERVER_PID" 2>/dev/null || { echo "server died:"; cat "$LOG"; exit 1; }
  sleep 0.2
done

AUTH="x-wakemate-token: $TOKEN"
echo "==> server up (pid $SERVER_PID)"
echo

echo "unauthenticated surface"
assert "GET /            reports running"      'WakeMATE companion is running' "$(curl -s "$BASE/")"
assert "GET /v1/health   reports online"       '"status":"online"'             "$(curl -s "$BASE/v1/health")"
assert "GET /v1/health   carries protocol v4"  '"protocol_version":4'          "$(curl -s "$BASE/v1/health")"
echo

echo "token hydration fell back to the config file (not the login keychain)"
assert "keyring reported unavailable"          'default keychain could not be found' "$(cat "$LOG")"
assert "config still holds the token"          "\"api_token\": \"$TOKEN\""     "$(cat "$CONFIG")"
echo

echo "authentication"
assert "GET /v1/info     401s without a token" '"message":"unauthorized"'      "$(curl -s "$BASE/v1/info")"
assert "GET /v1/info     401s on a bad token"  '"message":"unauthorized"'      "$(curl -s -H 'x-wakemate-token: wrong' "$BASE/v1/info")"
assert "GET /v1/info     200s with the token"  '"device_name":"SmokeTest'      "$(curl -s -H "$AUTH" "$BASE/v1/info")"
assert "GET /v1/pairing/check accepts token"   'pairing token accepted'        "$(curl -s -H "$AUTH" "$BASE/v1/pairing/check")"
echo
# NOTE: only 3 bad tokens above. The per-IP limiter locks out for 60s after
# 8 failures in a 60s window — do not add more negative auth cases here.

echo "TLS listener (self-signed, pinned by the phone via the QR fingerprint)"
assert "GET https /v1/health 200s with -k"     '"status":"online"'             "$(curl -sk "https://127.0.0.1:$TLS_PORT/v1/health")"
FP="$(echo | openssl s_client -connect "127.0.0.1:$TLS_PORT" 2>/dev/null | openssl x509 -noout -fingerprint -sha256)"
assert "certificate exposes a SHA-256 pin"     'sha256 Fingerprint='           "$FP"
echo

echo "pairing (macOS has no tray, so approval is refused by design)"
assert "GET  /v1/pairing/status is idle"       '"approval":"idle"'             "$(curl -s -H "$AUTH" "$BASE/v1/pairing/status")"
assert "POST /v1/pairing/enroll   403s"        'tray app is not running'       "$(curl -s -X POST -H "$AUTH" -H 'Content-Type: application/json' -d '{"device_name":"SmokePhone"}' "$BASE/v1/pairing/enroll")"
assert "POST /v1/pairing/activate 403s"        'tray app is not running'       "$(curl -s -X POST -H "$AUTH" "$BASE/v1/pairing/activate")"
echo

echo "wake-on-LAN"
assert "POST /v1/wake rejects a bad MAC"       'exactly 12 hex digits'         "$(curl -s -X POST -H "$AUTH" -H 'Content-Type: application/json' -d '{"mac":"not-a-mac"}' "$BASE/v1/wake")"
assert "POST /v1/wake sends a magic packet"    'wake packet sent'              "$(curl -s -X POST -H "$AUTH" -H 'Content-Type: application/json' -d '{"mac":"2C:F0:5D:59:89:44","broadcast":"255.255.255.255"}' "$BASE/v1/wake")"
assert "POST /v1/command wake form works"      'wake packet sent'              "$(curl -s -X POST -H "$AUTH" -H 'Content-Type: application/json' -d '{"type":"wake","mac":"2C:F0:5D:59:89:44","broadcast":"255.255.255.255"}' "$BASE/v1/command")"
echo

echo "capability gating"
if [ "$ENABLE_INPUT" -eq 1 ]; then
  # Real CGEvents: this nudges the actual cursor 5px. Harmless, but it is the
  # live desktop, not a sandbox.
  assert "mouse_move runs when input is on"    'mouse moved by 5, 0'           "$(curl -s -X POST -H "$AUTH" -H 'Content-Type: application/json' -d '{"type":"mouse_move","delta_x":5,"delta_y":0}' "$BASE/v1/command")"
  assert "media keys are macOS-unsupported"    'unsupported key: playpause'    "$(curl -s -X POST -H "$AUTH" -H 'Content-Type: application/json' -d '{"type":"media","action":"play_pause"}' "$BASE/v1/command")"
else
  assert "mouse_move 403s when input is off"   'input commands are disabled'   "$(curl -s -X POST -H "$AUTH" -H 'Content-Type: application/json' -d '{"type":"mouse_move","delta_x":5,"delta_y":5}' "$BASE/v1/command")"
fi
assert "system action 403s (power is off)"     'power commands are disabled'   "$(curl -s -X POST -H "$AUTH" -H 'Content-Type: application/json' -d '{"type":"system","action":"sleep"}' "$BASE/v1/command")"
assert "security_screen 403s (power is off)"   'power commands are disabled'   "$(curl -s -X POST -H "$AUTH" -H 'Content-Type: application/json' -d '{"type":"security_screen"}' "$BASE/v1/command")"
assert "unknown command type is rejected"      'unknown variant'               "$(curl -s -X POST -H "$AUTH" -H 'Content-Type: application/json' -d '{"type":"not_a_command"}' "$BASE/v1/command")"
echo

echo "===================================="
printf 'passed %d, failed %d\n' "$PASS" "$FAIL"
echo "server log: $LOG"

if [ "$KEEP_RUNNING" -eq 1 ]; then
  echo
  echo "server still running (pid $SERVER_PID). Talk to it with:"
  echo "  curl -s -H 'x-wakemate-token: $TOKEN' $BASE/v1/info"
  echo "  curl -sk https://127.0.0.1:$TLS_PORT/v1/health"
  echo "stop it with: kill $SERVER_PID"
fi

[ "$FAIL" -eq 0 ]
