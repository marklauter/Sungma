#!/usr/bin/env bash
# Starts Rauthy on port 9080 as Author's identity provider for local
# development, unless something already answers there. Its issuer is
# http://localhost:9080/auth/v1/.
#
#   scripts/rauthy.sh              start it, creating it on first use
#   podman stop sungma-rauthy      stop it
#   podman rm sungma-rauthy        forget it: the next start creates it afresh
#
# On first use it bootstraps an admin and the `author` client, with secrets
# it generates into .rauthy/: admin.secret holds the password for
# admin@localhost, and author.secret holds the client secret Author takes as
# SUNGMA_AUTHOR_CLIENT_SECRET. Rauthy keeps its data in the container, so
# people and settings last until the container is removed.
set -euo pipefail

image=ghcr.io/sebadob/rauthy:0.37.2@sha256:d733b7580f53c1601c286b5504853b2774e70245f5f9427d9bcda32496b753ee
name=sungma-rauthy
issuer=http://localhost:9080/auth/v1/
author=http://localhost:8090

repo="$(cd "$(dirname "$0")/.." && pwd -W 2>/dev/null || pwd)"
secrets="$repo/.rauthy"

answers() {
  curl -s -f -o /dev/null --max-time 2 "${issuer}.well-known/openid-configuration"
}

# Letters and digits, which every Rauthy secret accepts. tr fails when head
# stops reading, so its status is ignored.
random() {
  { LC_ALL=C tr -dc 'A-Za-z0-9' < /dev/urandom || true; } | head -c "$1"
}

create() {
  mkdir -p "$secrets/bootstrap"
  random 32 > "$secrets/admin.secret"
  random 64 > "$secrets/author.secret"
  cat > "$secrets/bootstrap/clients.json" <<EOF
[
  {
    "id": "author",
    "name": "Sungma Author",
    "secret": { "Plain": "$(cat "$secrets/author.secret")" },
    "redirect_uris": ["$author/auth/callback"],
    "post_logout_redirect_uris": ["$author/"],
    "enabled": true,
    "flows_enabled": ["authorization_code"],
    "access_token_alg": "EdDSA",
    "id_token_alg": "EdDSA",
    "auth_code_lifetime": 60,
    "access_token_lifetime": 1800,
    "scopes": ["openid", "profile", "email"],
    "default_scopes": ["openid", "profile", "email"],
    "challenges": ["S256"],
    "force_mfa": false
  }
]
EOF
  podman create --name "$name" -p 127.0.0.1:9080:8080 \
    -v "$secrets/bootstrap:/app/bootstrap:ro" \
    -e PUB_URL=localhost:9080 \
    -e LISTEN_SCHEME=http \
    -e LISTEN_PORT_HTTP=8080 \
    -e PROXY_MODE=false \
    -e COOKIE_MODE=danger-insecure \
    -e RP_ID=localhost \
    -e RP_ORIGIN=http://localhost:9080 \
    -e "RP_NAME=Sungma dev" \
    -e SUSPICIOUS_REQUESTS_BLACKLIST=0s \
    -e CRED_STUFF_BLACKLIST_THRES=1000000 \
    -e HIQLITE=true \
    -e HQL_NODE_ID=1 \
    -e "HQL_NODES=1 localhost:8100 localhost:8200" \
    -e HQL_INSECURE_COOKIE=true \
    -e "HQL_SECRET_RAFT=$(random 48)" \
    -e "HQL_SECRET_API=$(random 48)" \
    -e "ENC_KEYS=dev/$(head -c 32 /dev/urandom | base64)" \
    -e ENC_KEY_ACTIVE=dev \
    -e BOOTSTRAP_DIR=bootstrap \
    -e BOOTSTRAP_ADMIN_EMAIL=admin@localhost \
    -e "BOOTSTRAP_ADMIN_PASSWORD_PLAIN=$(cat "$secrets/admin.secret")" \
    "$image" serve > /dev/null
}

if answers; then
  exit 0
fi
"$(dirname "$0")/podman.sh"
if ! podman container exists "$name"; then
  create
fi
podman start "$name" > /dev/null
for _ in $(seq 1 60); do
  if answers; then
    exit 0
  fi
  sleep 1
done
echo "Rauthy didn't answer at $issuer" >&2
exit 1
