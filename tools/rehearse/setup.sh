#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
export UV_CACHE_DIR="$PWD/.rehearse/uv-cache"
if [[ ! -x .rehearse/venv/bin/python ]]; then
    uv venv --python python3 .rehearse/venv
fi
uv pip install --python .rehearse/venv/bin/python --torch-backend cpu -r tools/rehearse/requirements.txt
.rehearse/venv/bin/python tools/rehearse/rehearse.py setup
