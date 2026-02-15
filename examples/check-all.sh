#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

"${ROOT_DIR}/examples/integration-check.sh"
"${ROOT_DIR}/examples/proc-macro-check.sh"
"${ROOT_DIR}/examples/target-layout-check.sh"

echo "check-all: ok"
