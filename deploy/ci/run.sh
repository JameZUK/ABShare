#!/bin/sh
# Runs the CI deploy test against the image tagged abshare:ci.
set -eu
cd "$(dirname "$0")"
rm -rf work && mkdir -p work/library work/data work/certs
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj "/CN=books.example.com" \
  -addext "subjectAltName=DNS:books.example.com" -keyout work/certs/key.pem -out work/certs/cert.pem 2>/dev/null
chmod 0644 work/certs/key.pem
# The data directory belongs to the server's user (65534) with mode 0700.
# As root in a throwaway container: no sudo needed on the host. (The ABShare image has no
# shell, so a small pinned Alpine does this.)
ALPINE=alpine:3.24@sha256:294b683cb724975bec92580e1e685676bd4b50bda910ddb8c51d4cabeaec77e6
as_root() { docker run --rm --user 0 -v "$PWD/work:/w" --entrypoint /bin/sh "$ALPINE" -c "$1"; }
as_root 'chown 65534:65534 /w/data && chmod 0700 /w/data'
chmod a-w work/library
cleanup() {
  docker compose logs --no-color abshare | tail -20
  docker compose down -v >/dev/null 2>&1 || true
  as_root 'rm -rf /w/data' || true
  chmod u+w work/library && rm -rf work
}
trap cleanup EXIT
docker compose up -d --quiet-pull
for _ in $(seq 1 30); do
  curl -sk -o /dev/null --resolve books.example.com:8443:127.0.0.1 https://books.example.com:8443/healthz && break
  sleep 1
done
OUT=$(docker compose exec -T abshare /usr/local/bin/abshare admin invite --config /etc/abshare/abshare.toml 2>&1) || { echo "$OUT"; exit 1; }
INVITE=$(printf "%s\n" "$OUT" | grep -o "https://[^ ]*" | head -1)
[ -n "$INVITE" ] || { echo "no invite link in: $OUT"; exit 1; }
python3 -I test.py "$INVITE"
