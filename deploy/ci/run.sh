#!/bin/sh
# Runs the CI deploy test against the image tagged abshare:ci. Needs docker, openssl, curl,
# python3 and ffmpeg (which builds the invented demo library the image then scans and plays).
# ABSHARE_IMAGE names another tag to test.
set -eu
cd "$(dirname "$0")"
[ -d work/library ] && chmod -R u+w work/library
rm -rf work && mkdir -p work/data work/certs
# The synthetic demo library (invented titles and people), read-only as a real one must be.
../../tools/demo/make-library.sh work/library >/dev/null
chmod -R a-w work/library
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj "/CN=books.example.com" \
  -addext "subjectAltName=DNS:books.example.com" -keyout work/certs/key.pem -out work/certs/cert.pem 2>/dev/null
chmod 0644 work/certs/key.pem
# The data directory belongs to the server's user (65534) with mode 0700.
# As root in a throwaway container: no sudo needed on the host. (The ABShare image has no
# shell, so a small pinned Alpine does this.)
ALPINE=alpine:3.24@sha256:294b683cb724975bec92580e1e685676bd4b50bda910ddb8c51d4cabeaec77e6
as_root() { docker run --rm --user 0 -v "$PWD/work:/w" --entrypoint /bin/sh "$ALPINE" -c "$1"; }
as_root 'chown 65534:65534 /w/data && chmod 0700 /w/data'
cleanup() {
  docker compose logs --no-color abshare | tail -20
  docker compose --profile scan down -v >/dev/null 2>&1 || true
  as_root 'rm -rf /w/data' || true
  chmod -R u+w work/library && rm -rf work
}
trap cleanup EXIT

# The scan, in the image: its minimal ffprobe and ffmpeg run in the jail under Docker, with no
# network at all (as the scan service in deploy/compose.example.yaml).
SCAN=$(docker compose --profile scan run --rm scan 2>&1) || { echo "$SCAN"; echo "FAIL the scan in the image"; exit 1; }
printf '%s\n' "$SCAN" | tail -15
# The image's own cover renderer (abshare render-cover) ran in the jail: no cover failed.
if printf '%s\n' "$SCAN" | grep -q "\[cover\]"; then echo "FAIL a cover was not rendered in the image"; exit 1; fi
# doctor, in the image: the jailed ffmpeg and the jailed cover renderer both run.
DOCTOR=$(docker compose --profile scan run --rm scan doctor --config /etc/abshare/abshare.toml 2>&1) || { echo "$DOCTOR"; echo "FAIL doctor in the image"; exit 1; }
printf '%s\n' "$DOCTOR" | grep -q "\[  ok  \] jailed covers" || { echo "$DOCTOR"; echo "FAIL doctor did not check the jailed cover renderer"; exit 1; }
printf '%s\n' "$DOCTOR" | grep "jailed"

docker compose up -d --quiet-pull
for _ in $(seq 1 30); do
  curl -sk -o /dev/null --resolve books.example.com:8443:127.0.0.1 https://books.example.com:8443/healthz && break
  sleep 1
done
OUT=$(docker compose exec -T abshare /usr/local/bin/abshare admin invite --config /etc/abshare/abshare.toml 2>&1) || { echo "$OUT"; exit 1; }
INVITE=$(printf "%s\n" "$OUT" | grep -o "https://[^ ]*" | head -1)
[ -n "$INVITE" ] || { echo "no invite link in: $OUT"; exit 1; }
python3 -I test.py "$INVITE"

# The image's own health check (abshare health: there is no shell or curl in it).
ID=$(docker compose ps -q abshare)
HEALTH=starting
for _ in $(seq 1 30); do
  HEALTH=$(docker inspect -f '{{.State.Health.Status}}' "$ID")
  [ "$HEALTH" = starting ] || break
  sleep 1
done
[ "$HEALTH" = healthy ] || { echo "FAIL the container's health check says: $HEALTH"; exit 1; }
echo "ok   the container's health check passes"

# docker stop sends SIGTERM to PID 1: the server must stop by itself, promptly, with status 0
# (137 would mean Docker had to kill it after its grace period).
START=$(date +%s)
docker compose stop abshare >/dev/null 2>&1
TOOK=$(( $(date +%s) - START ))
CODE=$(docker inspect -f '{{.State.ExitCode}}' "$ID")
[ "$CODE" = 0 ] && [ "$TOOK" -le 5 ] || { echo "FAIL stopping took $TOOK s and ended with status $CODE"; exit 1; }
echo "ok   the server stops cleanly on SIGTERM ($TOOK s, status $CODE)"
