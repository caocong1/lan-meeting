#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

VM_NAME="${VM_NAME:-Windows 11}"
WINDOWS_REPO="${WINDOWS_REPO:-C:\\workspace\\lan-meeting}"
WINDOWS_IP="${WINDOWS_IP:-10.211.55.5}"
DURATION_MS="${DURATION_MS:-20000}"
FRAMES="${FRAMES:-30}"
DISPLAY_ID="${DISPLAY_ID:-0}"
FPS="${FPS:-30}"

TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
RUN_DIR="${ARTIFACT_ROOT:-artifacts/e2e-screen-probe}/${TIMESTAMP}"
WINDOWS_ARTIFACT_DIR="artifacts\\e2e-screen-probe\\${TIMESTAMP}"
mkdir -p "$RUN_DIR"

echo "[e2e-screen-probe] artifacts: $RUN_DIR"
echo "[e2e-screen-probe] building Mac viewer probe"
cargo build --manifest-path src-tauri/Cargo.toml --bin e2e_probe --no-default-features \
  > "$RUN_DIR/mac-build.log" \
  2>&1

echo "[e2e-screen-probe] starting Windows sharer probe in $VM_NAME"

prlctl exec "$VM_NAME" --current-user powershell -NoProfile -ExecutionPolicy Bypass \
  -File "${WINDOWS_REPO}\\scripts\\e2e-screen-probe-windows.ps1" \
  -Action start-sharer \
  -Repo "$WINDOWS_REPO" \
  -ArtifactDir "$WINDOWS_ARTIFACT_DIR" \
  -DurationMs "$((DURATION_MS + 5000))" \
  -DisplayId "$DISPLAY_ID" \
  -Fps "$FPS" \
  > "$RUN_DIR/windows-start.json"

echo "[e2e-screen-probe] waiting for Windows QUIC listener"
LISTENER_READY=0
for _ in $(seq 1 60); do
  prlctl exec "$VM_NAME" --current-user powershell -NoProfile -ExecutionPolicy Bypass \
    -File "${WINDOWS_REPO}\\scripts\\e2e-screen-probe-windows.ps1" \
    -Action collect \
    -Repo "$WINDOWS_REPO" \
    -ArtifactDir "$WINDOWS_ARTIFACT_DIR" \
    -StdoutName windows-sharer.log \
    > "$RUN_DIR/windows-sharer-startup.log" || true

  if grep -q "QUIC endpoint created" "$RUN_DIR/windows-sharer-startup.log"; then
    LISTENER_READY=1
    break
  fi

  sleep 1
done

if [[ "$LISTENER_READY" -ne 1 ]]; then
  echo "[e2e-screen-probe] Windows listener did not become ready within 60s" >&2
  exit 124
fi

echo "[e2e-screen-probe] running Mac headless viewer against ${WINDOWS_IP}:19876"
set +e
src-tauri/target/debug/e2e_probe \
  --mode viewer \
  --peer "$WINDOWS_IP" \
  --duration-ms "$DURATION_MS" \
  --frames "$FRAMES" \
  --display-id "$DISPLAY_ID" \
  --fps "$FPS" \
  --json \
  > "$RUN_DIR/mac-viewer.json" \
  2> "$RUN_DIR/mac-viewer.log"
VIEWER_STATUS=$?
set -e

echo "[e2e-screen-probe] collecting Windows probe logs"
prlctl exec "$VM_NAME" --current-user powershell -NoProfile -ExecutionPolicy Bypass \
  -File "${WINDOWS_REPO}\\scripts\\e2e-screen-probe-windows.ps1" \
  -Action collect \
  -Repo "$WINDOWS_REPO" \
  -ArtifactDir "$WINDOWS_ARTIFACT_DIR" \
  -StdoutName windows-sharer.json \
  > "$RUN_DIR/windows-sharer.json" || true

prlctl exec "$VM_NAME" --current-user powershell -NoProfile -ExecutionPolicy Bypass \
  -File "${WINDOWS_REPO}\\scripts\\e2e-screen-probe-windows.ps1" \
  -Action collect \
  -Repo "$WINDOWS_REPO" \
  -ArtifactDir "$WINDOWS_ARTIFACT_DIR" \
  -StdoutName windows-sharer.log \
  > "$RUN_DIR/windows-sharer.log" || true

prlctl exec "$VM_NAME" --current-user powershell -NoProfile -ExecutionPolicy Bypass \
  -File "${WINDOWS_REPO}\\scripts\\e2e-screen-probe-windows.ps1" \
  -Action collect \
  -Repo "$WINDOWS_REPO" \
  -ArtifactDir "$WINDOWS_ARTIFACT_DIR" \
  -StdoutName windows-build.log \
  > "$RUN_DIR/windows-build.log" || true

echo "[e2e-screen-probe] stopping any remaining Windows probe process"
prlctl exec "$VM_NAME" --current-user powershell -NoProfile -ExecutionPolicy Bypass \
  -File "${WINDOWS_REPO}\\scripts\\e2e-screen-probe-windows.ps1" \
  -Action stop \
  -Repo "$WINDOWS_REPO" \
  -ArtifactDir "$WINDOWS_ARTIFACT_DIR" \
  > "$RUN_DIR/windows-stop.json" || true

echo "[e2e-screen-probe] viewer exit status: $VIEWER_STATUS"
echo "[e2e-screen-probe] key files:"
echo "  $RUN_DIR/mac-viewer.json"
echo "  $RUN_DIR/mac-viewer.log"
echo "  $RUN_DIR/windows-sharer.json"
echo "  $RUN_DIR/windows-sharer.log"
echo "  $RUN_DIR/windows-build.log"

exit "$VIEWER_STATUS"
