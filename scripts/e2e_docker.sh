#!/usr/bin/env bash
# ==============================================================================
# Skoci — Docker End-to-End Test Suite
# Tests server startup, static fixture deploy, proxy routing, deny rules,
# container user privileges, redeployment cleanup, and Docker integration.
# ==============================================================================

set -euo pipefail

# ------------------------------------------------------------------------------
# 1. Configuration & Prerequisites
# ------------------------------------------------------------------------------

SKOCI_BIN="${SKOCI_BIN:-}"
if [ -z "$SKOCI_BIN" ]; then
  if [ -x "target/release/skoci" ]; then
    SKOCI_BIN="target/release/skoci"
  elif [ -x "target/debug/skoci" ]; then
    SKOCI_BIN="target/debug/skoci"
  elif command -v skoci >/dev/null 2>&1; then
    SKOCI_BIN="$(command -v skoci)"
  else
    SKOCI_BIN="target/release/skoci"
  fi
fi

# Every command below runs from the fixture directory, so the binary path must be absolute.
case "$SKOCI_BIN" in
  /*) ;;
  */*) SKOCI_BIN="$(cd "$(dirname "$SKOCI_BIN")" 2>/dev/null && pwd)/$(basename "$SKOCI_BIN")" ;;
  *) SKOCI_BIN="$(command -v "$SKOCI_BIN" 2>/dev/null || true)" ;;
esac

if [[ "$SKOCI_BIN" != /* ]]; then
  echo "Error: could not resolve the Skoci binary to an absolute path (SKOCI_BIN='$SKOCI_BIN')" >&2
  echo "Build it with 'cargo build --release' or set SKOCI_BIN to a full path." >&2
  exit 1
fi

if [ ! -x "$SKOCI_BIN" ]; then
  echo "Error: Skoci binary not found or not executable at '$SKOCI_BIN'" >&2
  echo "Build it with 'cargo build --release' or set SKOCI_BIN." >&2
  exit 1
fi

SKOCI_CONTROL_PORT="${SKOCI_CONTROL_PORT:-18787}"
SKOCI_HTTP_PORT="${SKOCI_HTTP_PORT:-18080}"
SKOCI_HTTPS_PORT="${SKOCI_HTTPS_PORT:-18443}"
MASTER_TOKEN="${SKOCI_MASTER_TOKEN:-skoci-e2e-master-token-minimum-32-chars-ok}"

if [ "${#MASTER_TOKEN}" -lt 32 ]; then
  echo "Error: SKOCI_MASTER_TOKEN must be at least 32 characters long" >&2
  exit 1
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "Error: docker binary not found in PATH" >&2
  exit 1
fi

if ! docker info >/dev/null 2>&1; then
  echo "Error: Docker daemon is not running or not accessible" >&2
  exit 1
fi

if ! command -v curl >/dev/null 2>&1; then
  echo "Error: curl binary not found in PATH" >&2
  exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
  echo "Error: python3 binary not found in PATH" >&2
  exit 1
fi

# ------------------------------------------------------------------------------
# 2. State & Cleanup Handler
# ------------------------------------------------------------------------------

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/skoci-e2e-work.XXXXXX")"
DATA_DIR="$(mktemp -d "${TMPDIR:-/tmp}/skoci-e2e-data.XXXXXX")"
DAEMON_LOG="${WORK_DIR}/daemon.log"
FIXTURE_DIR="${WORK_DIR}/fixture"

DAEMON_PID=""
APP_NAME="static-fixture-app"
FIRST_CONTAINER=""
SECOND_CONTAINER=""
IMAGE_TAG=""

cleanup() {
  local exit_code=$?
  set +e

  echo "Tearing down test resources..."

  if [ -n "$DAEMON_PID" ] && kill -0 "$DAEMON_PID" 2>/dev/null; then
    kill "$DAEMON_PID" 2>/dev/null || true
    for _ in $(seq 1 10); do
      if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
        break
      fi
      sleep 0.5
    done
    if kill -0 "$DAEMON_PID" 2>/dev/null; then
      kill -9 "$DAEMON_PID" 2>/dev/null || true
    fi
  fi

  if [ -n "$FIRST_CONTAINER" ]; then
    docker rm -f "$FIRST_CONTAINER" >/dev/null 2>&1 || true
  fi

  if [ -n "$SECOND_CONTAINER" ]; then
    docker rm -f "$SECOND_CONTAINER" >/dev/null 2>&1 || true
  fi

  leftover_containers=$(docker ps -a --filter "name=skoci-${APP_NAME}-" --format '{{.ID}}' 2>/dev/null || true)
  if [ -n "$leftover_containers" ]; then
    while IFS= read -r cid; do
      if [ -n "$cid" ]; then
        docker rm -f "$cid" >/dev/null 2>&1 || true
      fi
    done <<< "$leftover_containers"
  fi

  leftover_images=$(docker images --filter "reference=skoci/${APP_NAME}*" --format '{{.ID}}' 2>/dev/null || true)
  if [ -n "$leftover_images" ]; then
    while IFS= read -r iid; do
      if [ -n "$iid" ]; then
        docker rmi -f "$iid" >/dev/null 2>&1 || true
      fi
    done <<< "$leftover_images"
  fi

  docker network rm "skoci-net-${APP_NAME}" >/dev/null 2>&1 || true

  rm -rf "$WORK_DIR" "$DATA_DIR"

  if [ "$exit_code" -ne 0 ]; then
    echo "E2E test failed with exit code $exit_code." >&2
    if [ -f "$DAEMON_LOG" ]; then
      echo "=== Daemon Log Tail ===" >&2
      tail -n 50 "$DAEMON_LOG" >&2
    fi
  fi

  exit "$exit_code"
}
trap cleanup EXIT INT TERM

# ------------------------------------------------------------------------------
# 3. Start Skoci Daemon
# ------------------------------------------------------------------------------

echo "==> [1/6] Starting Skoci daemon..."
export SKOCI_BIND="127.0.0.1:${SKOCI_CONTROL_PORT}"
export SKOCI_DATA_DIR="${DATA_DIR}"
export SKOCI_HTTP_BIND="127.0.0.1:${SKOCI_HTTP_PORT}"
export SKOCI_HTTPS_BIND="127.0.0.1:${SKOCI_HTTPS_PORT}"
export SKOCI_CONTROL_TLS_BIND="off"
export SKOCI_TLS="off"
export SKOCI_MASTER_TOKEN="${MASTER_TOKEN}"

# For client commands
export SKOCI_SERVER_URL="http://127.0.0.1:${SKOCI_CONTROL_PORT}"
export SKOCI_TOKEN="${MASTER_TOKEN}"

"$SKOCI_BIN" server run \
  --bind "127.0.0.1:${SKOCI_CONTROL_PORT}" \
  --data-dir "${DATA_DIR}" \
  --http-bind "127.0.0.1:${SKOCI_HTTP_PORT}" \
  --https-bind "127.0.0.1:${SKOCI_HTTPS_PORT}" \
  --control-tls-bind "off" \
  --tls off > "$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!

# ------------------------------------------------------------------------------
# 4. Wait for /api/health
# ------------------------------------------------------------------------------

echo "==> [2/6] Waiting for Skoci daemon /api/health..."
HEALTH_URL="http://127.0.0.1:${SKOCI_CONTROL_PORT}/api/health"
HEALTHY=0
for _ in $(seq 1 30); do
  if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
    echo "Error: Skoci daemon exited prematurely." >&2
    cat "$DAEMON_LOG" >&2
    exit 1
  fi

  if response=$(curl -fsS "$HEALTH_URL" 2>/dev/null); then
    if echo "$response" | grep -q '"ok"[[:space:]]*:[[:space:]]*true'; then
      HEALTHY=1
      break
    fi
  fi
  sleep 1
done

if [ "$HEALTHY" -ne 1 ]; then
  echo "Error: Skoci daemon did not become healthy within 30 seconds." >&2
  cat "$DAEMON_LOG" >&2
  exit 1
fi
echo "✓ Skoci daemon is healthy."

# ------------------------------------------------------------------------------
# 5. Create Static Fixture Project
# ------------------------------------------------------------------------------

echo "==> [3/6] Setting up static fixture project..."
mkdir -p "${FIXTURE_DIR}/data"

cat <<'EOF' > "${FIXTURE_DIR}/index.html"
<!DOCTYPE html>
<html lang="en">
<head><meta charset="utf-8"><title>Skoci Static Fixture</title></head>
<body><h1>Hello from Skoci Static Fixture</h1></body>
</html>
EOF

cat <<'EOF' > "${FIXTURE_DIR}/PRD.md"
# Product Requirements Document
Confidential internal specification document.
EOF

cat <<'EOF' > "${FIXTURE_DIR}/package.json"
{
  "name": "static-fixture-app",
  "version": "1.0.0",
  "private": true
}
EOF

cat <<'EOF' > "${FIXTURE_DIR}/data/items.json"
{
  "items": [
    {"id": 1, "name": "widget"},
    {"id": 2, "name": "gadget"}
  ]
}
EOF

(
  cd "$FIXTURE_DIR" || exit 1
  "$SKOCI_BIN" init
)

if [ ! -f "${FIXTURE_DIR}/.skoci/skoci.yaml" ]; then
  echo "Assertion failed: .skoci/skoci.yaml was not generated by skoci init" >&2
  exit 1
fi

if [ ! -f "${FIXTURE_DIR}/.skoci/Dockerfile" ]; then
  echo "Assertion failed: .skoci/Dockerfile was not generated by skoci init" >&2
  exit 1
fi

if ! grep -q "nginx" "${FIXTURE_DIR}/.skoci/Dockerfile"; then
  echo "Assertion failed: .skoci/Dockerfile does not contain expected nginx template" >&2
  exit 1
fi
echo "✓ Static fixture initialized with nginx template."

# ------------------------------------------------------------------------------
# 6. Deploy Project & Assert Live Status
# ------------------------------------------------------------------------------

echo "==> [4/6] Deploying static fixture project..."
(
  cd "$FIXTURE_DIR" || exit 1
  "$SKOCI_BIN" deploy
)

STATUS_OUTPUT=$("$SKOCI_BIN" status "$APP_NAME" --json)
CONTAINER_STATUS=$(echo "$STATUS_OUTPUT" | python3 -c 'import sys, json; data=json.load(sys.stdin); envs=data.get("data", {}).get("environments", []); print(envs[0].get("container_status", "") if envs else "")')
FIRST_CONTAINER=$(echo "$STATUS_OUTPUT" | python3 -c 'import sys, json; data=json.load(sys.stdin); envs=data.get("data", {}).get("environments", []); print(envs[0].get("container", "") if envs else "")')
APP_HEALTHY=$(echo "$STATUS_OUTPUT" | python3 -c 'import sys, json; data=json.load(sys.stdin); envs=data.get("data", {}).get("environments", []); print(str(envs[0].get("healthy", False)).lower() if envs else "false")')

if [ "$CONTAINER_STATUS" != "running" ]; then
  echo "Assertion failed: container status is '$CONTAINER_STATUS' (expected 'running')" >&2
  exit 1
fi

if [ "$APP_HEALTHY" != "true" ]; then
  echo "Assertion failed: deployment status is not healthy (healthy=$APP_HEALTHY)" >&2
  exit 1
fi

if [ -z "$FIRST_CONTAINER" ]; then
  echo "Assertion failed: could not determine container name from skoci status" >&2
  exit 1
fi

DEPLOY_REC_STATUS=$(curl -fsS -H "Authorization: Bearer $MASTER_TOKEN" "http://127.0.0.1:${SKOCI_CONTROL_PORT}/api/deployments?app=${APP_NAME}" | python3 -c 'import sys, json; data=json.load(sys.stdin); deps=data.get("data", []); print(deps[0].get("status", "") if deps else "")')

if [ "$DEPLOY_REC_STATUS" != "live" ]; then
  echo "Assertion failed: deployment record status is '$DEPLOY_REC_STATUS' (expected 'live')" >&2
  exit 1
fi
echo "✓ Deployment reached live status: container=$FIRST_CONTAINER"

# ------------------------------------------------------------------------------
# 7. Assert Serving, Deny Rules, Image Contents, and Non-Root User
# ------------------------------------------------------------------------------

echo "==> [5/6] Verifying proxy endpoints, deny rules, image contents, and non-root execution..."
PROXY_BASE="http://127.0.0.1:${SKOCI_HTTP_PORT}"

# Assertion 7a: GET / is 200 through proxy
STATUS_ROOT=$(curl -s -o /dev/null -w "%{http_code}" "${PROXY_BASE}/")
if [ "$STATUS_ROOT" != "200" ]; then
  echo "Assertion failed: GET / returned HTTP $STATUS_ROOT (expected 200)" >&2
  exit 1
fi

ROOT_BODY=$(curl -fsS "${PROXY_BASE}/")
if ! echo "$ROOT_BODY" | grep -q "Hello from Skoci Static Fixture"; then
  echo "Assertion failed: GET / body does not contain expected fixture text" >&2
  exit 1
fi
echo "✓ GET / returned 200 with fixture content"

# Assertion 7b: /healthz is 200
STATUS_HEALTHZ=$(curl -s -o /dev/null -w "%{http_code}" "${PROXY_BASE}/healthz")
if [ "$STATUS_HEALTHZ" != "200" ]; then
  echo "Assertion failed: GET /healthz returned HTTP $STATUS_HEALTHZ (expected 200)" >&2
  exit 1
fi
echo "✓ GET /healthz returned 200"

# Assertion 7c: /PRD.md is 404 (0.5.0 deny rule for .md)
STATUS_PRD=$(curl -s -o /dev/null -w "%{http_code}" "${PROXY_BASE}/PRD.md")
if [ "$STATUS_PRD" != "404" ]; then
  echo "Assertion failed: GET /PRD.md returned HTTP $STATUS_PRD (expected 404 deny rule)" >&2
  exit 1
fi
echo "✓ GET /PRD.md returned 404 (deny rule enforced)"

# Assertion 7d: /package.json is 404 (0.5.0 deny rule for package.json)
STATUS_PKG=$(curl -s -o /dev/null -w "%{http_code}" "${PROXY_BASE}/package.json")
if [ "$STATUS_PKG" != "404" ]; then
  echo "Assertion failed: GET /package.json returned HTTP $STATUS_PKG (expected 404 deny rule)" >&2
  exit 1
fi
echo "✓ GET /package.json returned 404 (deny rule enforced)"

# Assertion 7e: /data/items.json is 200
STATUS_DATA=$(curl -s -o /dev/null -w "%{http_code}" "${PROXY_BASE}/data/items.json")
if [ "$STATUS_DATA" != "200" ]; then
  echo "Assertion failed: GET /data/items.json returned HTTP $STATUS_DATA (expected 200)" >&2
  exit 1
fi

DATA_BODY=$(curl -fsS "${PROXY_BASE}/data/items.json")
if ! echo "$DATA_BODY" | grep -q "widget"; then
  echo "Assertion failed: GET /data/items.json body does not contain expected data" >&2
  exit 1
fi
echo "✓ GET /data/items.json returned 200 with data content"

# Assertion 7f: Image does not contain .skoci directory
IMAGE_TAG=$(docker inspect --format '{{.Config.Image}}' "$FIRST_CONTAINER")
if [ -z "$IMAGE_TAG" ]; then
  echo "Assertion failed: failed to extract image tag from container $FIRST_CONTAINER" >&2
  exit 1
fi

if docker run --rm "$IMAGE_TAG" test -e /usr/share/nginx/html/.skoci; then
  echo "Assertion failed: Docker image $IMAGE_TAG contains /usr/share/nginx/html/.skoci" >&2
  exit 1
fi
echo "✓ Docker image $IMAGE_TAG does not contain .skoci directory"

# Assertion 7g: Container runs as a non-root user
CONTAINER_CONFIG_USER=$(docker inspect --format '{{.Config.User}}' "$FIRST_CONTAINER")
if [ "$CONTAINER_CONFIG_USER" = "root" ] || [ "$CONTAINER_CONFIG_USER" = "0" ] || [ -z "$CONTAINER_CONFIG_USER" ]; then
  echo "Assertion failed: container $FIRST_CONTAINER user is configured as '$CONTAINER_CONFIG_USER' (expected non-root)" >&2
  exit 1
fi

RUNNING_UID=$(docker exec "$FIRST_CONTAINER" id -u)
if [ "$RUNNING_UID" = "0" ]; then
  echo "Assertion failed: container $FIRST_CONTAINER is executing with UID 0 (root)" >&2
  exit 1
fi
echo "✓ Container runs as non-root user (Config.User='$CONTAINER_CONFIG_USER', UID=$RUNNING_UID)"

# ------------------------------------------------------------------------------
# 8. Second Deploy & Orphan Container Cleanup Check
# ------------------------------------------------------------------------------

echo "==> [6/6] Running second deploy and asserting previous container cleanup..."

cat <<'EOF' > "${FIXTURE_DIR}/index.html"
<!DOCTYPE html>
<html lang="en">
<head><meta charset="utf-8"><title>Skoci Static Fixture v2</title></head>
<body><h1>Hello from Skoci Static Fixture v2</h1></body>
</html>
EOF

# Ensure version timestamp increment (1-second granularity in Skoci)
sleep 2

(
  cd "$FIXTURE_DIR" || exit 1
  "$SKOCI_BIN" deploy
)

SECOND_STATUS_OUTPUT=$("$SKOCI_BIN" status "$APP_NAME" --json)
SECOND_CONTAINER=$(echo "$SECOND_STATUS_OUTPUT" | python3 -c 'import sys, json; data=json.load(sys.stdin); envs=data.get("data", {}).get("environments", []); print(envs[0].get("container", "") if envs else "")')

if [ -z "$SECOND_CONTAINER" ]; then
  echo "Assertion failed: could not determine second container name from skoci status" >&2
  exit 1
fi

if [ "$SECOND_CONTAINER" = "$FIRST_CONTAINER" ]; then
  echo "Assertion failed: second deploy did not produce a new container name ($SECOND_CONTAINER == $FIRST_CONTAINER)" >&2
  exit 1
fi

# Assert previous container is gone (stopped and removed)
if docker inspect "$FIRST_CONTAINER" >/dev/null 2>&1; then
  echo "Assertion failed: previous container $FIRST_CONTAINER still exists in docker inspect after second deploy" >&2
  exit 1
fi

if docker ps -a --filter "name=^/${FIRST_CONTAINER}$" --format '{{.Names}}' | grep -q "^${FIRST_CONTAINER}$"; then
  echo "Assertion failed: previous container $FIRST_CONTAINER found in docker ps -a (orphan container detected)" >&2
  exit 1
fi

SECOND_RUNNING=$(docker inspect --format '{{.State.Running}}' "$SECOND_CONTAINER")
if [ "$SECOND_RUNNING" != "true" ]; then
  echo "Assertion failed: second container $SECOND_CONTAINER is not running (State.Running=$SECOND_RUNNING)" >&2
  exit 1
fi

V2_BODY=$(curl -fsS "${PROXY_BASE}/")
if ! echo "$V2_BODY" | grep -q "v2"; then
  echo "Assertion failed: GET / did not serve updated v2 content after second deploy" >&2
  exit 1
fi

echo "✓ Second deploy succeeded: new container=$SECOND_CONTAINER, previous container=$FIRST_CONTAINER cleaned up (no orphan)"
echo "==> All Docker end-to-end assertions passed successfully!"
