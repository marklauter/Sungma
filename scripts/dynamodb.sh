#!/usr/bin/env bash
# Starts DynamoDB Local on port 8000 for the sungma-dynamodb tests, unless
# something already answers there. In memory, so a restart starts empty;
# each test makes a table of its own. CI runs the same image.
#
#   scripts/dynamodb.sh          start it if it isn't running
#   podman stop sungma-dynamodb  stop it
set -euo pipefail

image=docker.io/amazon/dynamodb-local:3.3.1@sha256:744bf7138f0e893a6438de89bc3374501105b42bc7473cef6d561f30ff2dec6a
endpoint=http://localhost:8000

answers() {
  curl -s -o /dev/null --max-time 2 "$endpoint"
}

if answers; then
  exit 0
fi
podman run -d --rm --name sungma-dynamodb -p 8000:8000 "$image" \
  -jar DynamoDBLocal.jar -inMemory -sharedDb > /dev/null
for _ in $(seq 1 30); do
  if answers; then
    exit 0
  fi
  sleep 1
done
echo "DynamoDB Local didn't answer at $endpoint" >&2
exit 1
