#!/usr/bin/env python3
"""Mutation check (SPEC §13): removes each security protection in turn and confirms that at
least one test fails. Every file is restored afterwards, whatever happens.

usage: python3 tools/mutation-check.py   (from the repository root)
"""
import pathlib
import subprocess
import sys

MUTATIONS = [
    ("TLS enforcement", "server/src/http/guard.rs", "if !is_tls(peer, headers, &cfg.trusted) {", "if false && !is_tls(peer, headers, &cfg.trusted) {"),
    ("Host check", "server/src/http/guard.rs", "if !host_ok {", "if false && !host_ok {"),
    ("CSRF header and Origin", "server/src/http/guard.rs", "if !marked || !origin_ok {", "if false && (!marked || !origin_ok) {"),
    ("rightmost X-Forwarded-For", "server/src/http/guard.rs", "hops.into_iter().rev().find(", "hops.into_iter().find("),
    ("security headers", "server/src/http/guard.rs", "pub fn headers_on(mut r: Response) -> Response {", "pub fn headers_on(r: Response) -> Response {\n    #[allow(unreachable_code)]\n    return r;\n    let mut r = r;"),
    ("TOTP replay (core)", "core/src/auth/totp.rs", "Some(step) if last_used_step.is_some_and(|last| step <= last) => Err(TotpError::Replayed),", ""),
    ("TOTP replay (store race guard)", "server/src/auth/store.rs", "AND (totp_last_step IS NULL OR totp_last_step < ?2)", ""),
    ("refresh reuse detection", "server/src/auth/store.rs", "if now - used > REUSE_GRACE || used_children > 0 || children >= MAX_SIBLINGS {", "if false {"),
    ("step-up requirement", "server/src/http/me_routes.rs", "if st.sessions.stepped_up(s.device, now()) { Ok(()) } else { Err(ApiError::step_up()) }", "Ok(())"),
    ("address lockout", "core/src/auth/lockout.rs", "if a.locked_until > now {", "if false {"),
    ("last sign-in method", "server/src/http/me_routes.rs", "passkeys_left > 0 || (password && totp)", "true"),
    ("device ownership", "server/src/auth/store.rs", "WHERE id = ?1 AND user_id = ?2 AND revoked_at IS NULL", "WHERE id = ?1 AND revoked_at IS NULL"),
    ("single-use invites", "server/src/auth/store.rs", "\"UPDATE invite SET used_at = ?2 WHERE hash = ?1 AND used_at IS NULL AND expires_at > ?2\"", "\"UPDATE invite SET used_at = ?2 WHERE hash = ?1\""),
    ("logout revokes the session", "server/src/http/auth_routes.rs", "            st.sessions.revoke_device(device);\n            st.audit(\"logout\"", "            st.audit(\"logout\""),
    ("enrolment needs a factor", "server/src/http/auth_routes.rs", "if !has_passkey && !has_pw_totp {", "if false {"),
    ("recovery codes single use", "server/src/auth/store.rs", "WHERE user_id = ?1 AND hash = ?2 AND used_at IS NULL", "WHERE user_id = ?1 AND hash = ?2"),
]


def main() -> int:
    root = pathlib.Path(__file__).resolve().parent.parent
    survived = []
    for name, rel, find, replace in MUTATIONS:
        path = root / rel
        original = path.read_text()
        if find not in original:
            print(f"?? {name}: pattern not found in {rel} (update this script)")
            survived.append(name)
            continue
        try:
            path.write_text(original.replace(find, replace, 1))
            r = subprocess.run(
                ["cargo", "test", "-q", "-p", "abshare-core", "-p", "abshare-server"],
                cwd=root, capture_output=True, text=True,
            )
            caught = r.returncode != 0
            print(f"{'caught  ' if caught else 'SURVIVED'} {name}")
            if not caught:
                survived.append(name)
        finally:
            path.write_text(original)
    print(f"\n{len(MUTATIONS) - len(survived)}/{len(MUTATIONS)} mutations caught")
    return 1 if survived else 0


if __name__ == "__main__":
    sys.exit(main())
