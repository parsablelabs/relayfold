#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  cat <<'EOF'
Usage: scripts/run-local-dev.sh [--skip-build] [--worker-image IMAGE]
Run the orchestrator on the host and two workers in Docker Compose.
  --skip-build          Reuse the existing orchestrator binary.
  --worker-image IMAGE  Use a prebuilt worker image.
Uses run-orchestrator-dev.sh environment settings. Ctrl-C stops the workers.
EOF
}

skip_build=false
worker_image=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --skip-build)
      skip_build=true
      shift
      ;;
    --worker-image)
      if [[ -z "${2:-}" || "$2" == -* ]]; then
        echo "--worker-image requires an image reference" >&2
        exit 2
      fi
      worker_image="$2"
      shift 2
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
done
if [[ "$skip_build" == true ]]; then
  set -- --skip-build
fi

if [[ "$(uname -s)" == "Darwin" ]]; then
  export RELAYFOLD_ORCHESTRATOR_HTTP_URL="${RELAYFOLD_ORCHESTRATOR_HTTP_URL:-http://host.docker.internal:3001}"
fi

compose=(docker compose -f "$repo_root/docker-compose.yml" -f "$repo_root/docker-compose.local-workers.yml")
worker_build=--build
if [[ -n "$worker_image" ]]; then
  export RELAYFOLD_WORKER_IMAGE="$worker_image"
  compose+=(-f "$repo_root/docker-compose.worker-image.yml")
  worker_build=--no-build
fi
cleanup() {
  "${compose[@]}" stop worker
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

"${compose[@]}" up -d "$worker_build" --no-deps worker
"$repo_root/scripts/run-orchestrator-dev.sh" "$@"
