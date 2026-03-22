#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

"$repo_root/scripts/run-reference-parity.sh"
"$repo_root/scripts/run-reference-benchmarks.sh"
