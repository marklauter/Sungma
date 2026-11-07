#!/usr/bin/env bash
# Starts the Podman machine unless Podman already answers. The other scripts
# run it before they use Podman. On Linux, Podman runs without a machine, so
# it always answers.
#
#   scripts/podman.sh          start it if it isn't running
#   podman machine stop        stop it, freeing its memory
set -euo pipefail

if podman info > /dev/null 2>&1; then
  exit 0
fi
podman machine start > /dev/null
