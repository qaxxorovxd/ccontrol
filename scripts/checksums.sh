#!/usr/bin/env bash
# Write SHA256SUMS for every artifact in a directory.
set -euo pipefail
cd "${1:-dist}"
sha256sum ccontrol-* > SHA256SUMS
cat SHA256SUMS
