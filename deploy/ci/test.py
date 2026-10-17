"""CI deploy test: the release image behind nginx (SPEC §11 Phase 5 gate).

Checks, through the proxy: plain HTTP redirects to HTTPS; exactly one Content-Security-Policy
and HSTS on responses; a wrong Host is refused; the API refuses requests without a token;
enrolment works and its cookies are HttpOnly, Secure, SameSite=Strict with narrow paths; a
client can't dodge the sign-in lockout by spoofing X-Forwarded-For. Standard library only.

usage: test.py INVITE_LINK
"""
import base64
import hashlib
import hmac
import http.client
import json
import ssl
import struct
import sys
import time

HOST = "books.example.com"
ORIGIN = f"https://{HOST}"
CTX = ssl.create_default_context()
CTX.check_hostname = False
CTX.verify_mode = ssl.CERT_NONE
failures = []


def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what)
    if not cond:
        failures.append(what)


def request(method, path, body=None, headers=None, host=HOST, tls=True):
    conn = (http.client.HTTPSConnection("127.0.0.1", 8443, context=CTX) if tls else http.client.HTTPConnection("127.0.0.1", 8080))
    h = {"Host": host, "X-ABShare-Client": "1", "Origin": ORIGIN}
    if body is not None:
        h["Content-Type"] = "application/json"
        body = json.dumps(body)
    h.update(headers or {})
    conn.request(method, path, body=body, headers=h)
    r = conn.getresponse()
    data = r.read()
    conn.close()
    return r, data


def totp(secret):
    key = base64.b32decode(secret + "=" * (-len(secret) % 8))
    mac = hmac.new(key, struct.pack(">Q", int(time.time()) // 30), hashlib.sha1).digest()
    o = mac[-1] & 15
    return "%06d" % ((struct.unpack(">I", mac[o:o + 4])[0] & 0x7FFFFFFF) % 1_000_000)


def main(invite):
    r, _ = request("GET", "/", tls=False)
    check(r.status == 301 and r.getheader("Location", "").startswith("https://"), "plain HTTP redirects to HTTPS")

    r, body = request("GET", "/")
    csp = r.headers.get_all("Content-Security-Policy") or []
    check(r.status == 200 and b"<!doctype html>" in body.lower(), "the app is served over HTTPS")
    check(len(csp) == 1, f"exactly one Content-Security-Policy header (got {len(csp)})")
    check("require-trusted-types-for 'script'" in (csp[0] if csp else ""), "the CSP enforces Trusted Types")
    check("max-age=" in (r.getheader("Strict-Transport-Security") or ""), "HSTS is sent")
    check(r.getheader("X-Frame-Options") == "DENY", "framing is refused")

    r, _ = request("GET", "/", host="evil.example")
    check(r.status == 421, f"a wrong Host is refused with 421 (got {r.status})")

    for path in ("/v1/books", "/v1/me", "/v1/admin/users", "/v1/home"):
        r, _ = request("GET", path)
        check(r.status == 401, f"{path} needs a token (got {r.status})")
    r, _ = request("GET", "/v1/media/aaaaaaaaaaaaaaaaaaaaaaaaaa")
    check(r.status == 401, f"media needs a credential (got {r.status})")
    r, _ = request("POST", "/v1/auth/refresh", headers={"X-ABShare-Client": ""})
    check(r.status in (401, 403), "cookie routes need the CSRF header")

    # Enrolment through the proxy, then the cookies' flags.
    token = invite.rsplit("/", 1)[-1]
    r, b = request("POST", "/v1/auth/enrol/start", {"token": token, "username": "ci-user"})
    enrol = json.loads(b)["enrol"]
    r, b = request("POST", "/v1/auth/enrol/password", {"enrol": enrol, "password": "a long ci pass phrase"})
    secret = json.loads(b)["secret"]
    r, _ = request("POST", "/v1/auth/enrol/totp", {"enrol": enrol, "code": totp(secret)})
    r, b = request("POST", "/v1/auth/enrol/finish", {"enrol": enrol, "device_name": "CI"})
    check(r.status == 200, f"enrolment works through the proxy (got {r.status})")
    cookies = r.headers.get_all("Set-Cookie") or []
    for name, path in (("abshare_refresh", "/v1/auth"), ("abshare_media", "/v1/media")):
        c = next((c for c in cookies if c.startswith(name + "=")), "")
        flags = [f.strip() for f in c.split(";")]
        check("HttpOnly" in flags and "Secure" in flags and "SameSite=Strict" in flags and f"Path={path}" in flags,
              f"{name} is HttpOnly, Secure, SameSite=Strict, Path={path}")
    access = json.loads(b)["access_token"]
    r, _ = request("GET", "/v1/books", headers={"Authorization": f"Bearer {access}"})
    check(r.status == 200, "the API answers with a token")

    # Spoofed X-Forwarded-For can't spread failures over fake addresses: nginx overwrites it,
    # and ABShare trusts only nginx, so the sixth failure from one client is locked out.
    statuses = []
    for i in range(7):
        r, _ = request("POST", "/v1/auth/login/password",
                       {"username": f"nobody{i}", "password": "wrong password here", "device_name": "x"},
                       headers={"X-Forwarded-For": f"203.0.113.{i + 1}"})
        statuses.append(r.status)
    check(429 in statuses, f"spoofed X-Forwarded-For doesn't dodge the lockout (statuses {statuses})")

    if failures:
        print(f"{len(failures)} check(s) failed")
        sys.exit(1)
    print("all deploy checks passed")


if __name__ == "__main__":
    main(sys.argv[1])
