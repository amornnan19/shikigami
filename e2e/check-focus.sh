#!/bin/sh
set -u
root=$(cd "$(dirname "$0")/.." && pwd)
frontmost() {
  timeout 5 osascript -e 'tell application "System Events" to get name of first process whose frontmost is true'
}
before=$(frontmost)
echo "frontmost before: $before"
cd "$root" || exit 1
timeout 900 bun run e2e >"$root/e2e/artifacts/check-focus-run.log" 2>&1 &
pid=$!
samples=0
bad=0
while kill -0 "$pid" 2>/dev/null; do
  name=$(frontmost)
  samples=$((samples + 1))
  echo "sample $samples: $name"
  if [ "$name" = "shikigami" ]; then
    bad=$((bad + 1))
  fi
  sleep 1
done
wait "$pid"
status=$?
echo "samples: $samples, shikigami frontmost: $bad, e2e exit: $status"
if [ "$bad" -ne 0 ] || [ "$status" -ne 0 ] || [ "$samples" -eq 0 ]; then
  echo "FAIL"
  exit 1
fi
echo "PASS"
