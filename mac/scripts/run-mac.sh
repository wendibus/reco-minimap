#!/bin/zsh
# Developer run: builds the payload (program + interface) and starts the launcher against it.
set -euo pipefail
cd "${0:A:h}/.."
PAYLOAD_OUT="${TMPDIR:-/tmp}/reco-minimap-dev-payload"
RECO_SKIP_NPM="${RECO_SKIP_NPM:-}" ./scripts/build-payload.sh "$PAYLOAD_OUT" >/dev/null
RECO_MINIMAP_PAYLOAD_DIR="$PAYLOAD_OUT/payload" swift run RecoMinimapMac
