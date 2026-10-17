#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

case "${1:-}" in
  -h|--help)
    echo "Usage: scripts/run-local-dev.sh [--skip-build]"
    echo "Run the orchestrator on the host and two workers in Docker Compose."
    echo "Uses run-orchestrator-dev.sh environment settings; Ctrl-C stops the workers."
    exit 0
    ;;
  ''|--skip-build) ;;
  *) echo "Usage: scripts/run-local-dev.sh [--skip-build]" >&2; exit 2 ;;
esac
if [[ $# -gt 1 ]]; then
  echo "Usage: scripts/run-local-dev.sh [--skip-build]" >&2
  exit 2
fi

compose=(docker compose -f "$repo_root/docker-compose.yml" -f "$repo_root/docker-compose.local-workers.yml")
cleanup() {
  "${compose[@]}" stop worker
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

"${compose[@]}" up -d --build --no-deps worker
"$repo_root/scripts/run-orchestrator-dev.sh" "$@"
