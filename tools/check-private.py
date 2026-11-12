#!/usr/bin/env python3
"""Refuses commits and pushes that contain private information (SPEC §14).

Two kinds of check:

1. Generic checks, which need no list and always run:
   - IPv4 addresses, other than the placeholders and documentation ranges in ALLOWED_NETS;
   - email addresses, other than example and no-reply ones;
   - host names ending in a private suffix (.lan, .local, .internal, .home.arpa).
2. Your own private strings, one per line in the gitignored file
   dev/local.private-patterns.txt (real host names, addresses, library titles, folder names,
   emails...), or in the file named by ABSHARE_PRIVATE_PATTERNS. This script contains none of
   them. Lines starting with # are comments. Strings shorter than 5 characters aren't checked
   (they match everywhere); the script says so, by line number.

Without the list only the generic checks run, and the script says so: loudly in the hook
modes, where the list should exist; CI has no list and runs the generic checks alone.

usage:
  tools/check-private.py staged         # what is staged, binaries included (pre-commit hook)
  tools/check-private.py message FILE   # a commit message (commit-msg hook)
  tools/check-private.py range A..B     # commits about to be pushed (pre-push hook)
  tools/check-private.py tree           # every tracked file, binaries included (CI)
  tools/check-private.py all            # the whole history (text changes and messages)
  tools/check-private.py --self-test    # this script's own tests

The hooks are in .githooks; turn them on once with: git config core.hooksPath .githooks
"""
import ipaddress
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
MIN_PATTERN = 5

# Addresses that may appear: placeholders, documentation ranges, and addresses that say
# nothing about anyone's network.
ALLOWED_NETS = [ipaddress.ip_network(n) for n in (
    "10.0.0.0/24",      # the repository's placeholder range (10.0.0.10)
    "10.89.0.0/24",     # the CI deploy test's own Docker network (deploy/ci/compose.yaml)
    "127.0.0.0/8",      # loopback
    "0.0.0.0/32",       # "every address"
    "192.0.2.0/24",     # TEST-NET-1 (RFC 5737)
    "198.51.100.0/24",  # TEST-NET-2
    "203.0.113.0/24",   # TEST-NET-3
    "255.0.0.0/8",      # netmasks and broadcast
)]
IPV4 = re.compile(r"(?<![0-9A-Za-z_.\-])(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})(?![0-9A-Za-z_\-]|\.[0-9])")
# What comes just before a dotted version number rather than an address: "Chrome/120.0.0.0",
# "version 1.2.3.4", "v1.2.3.4", "@1.2.3.4".
VERSION_BEFORE = re.compile(r"(?:[A-Za-z]/|\bv|@|\bversion[ =:\"']{1,3}|\bfirefox |\bchrome |\bsafari )$", re.I)

EMAIL = re.compile(r"(?<![A-Za-z0-9._%+\-])([A-Za-z0-9][A-Za-z0-9._%+\-]*)@((?:[A-Za-z0-9\-]+\.)+[A-Za-z]{2,24})(?![A-Za-z0-9\-])")
EMAIL_OK_DOMAINS = ("example.com", "example.org", "example.net", "example", "invalid", "test",
                    "users.noreply.github.com")
EMAIL_OK_LOCAL = ("noreply", "no-reply")
# "git@github.com:owner/repo" is a clone address, not a mailbox.
EMAIL_NOT_MAIL = {"git@github.com", "git@gitlab.com"}

PRIVATE_HOST = re.compile(
    r"(?<![A-Za-z0-9_.\-$@{])((?:[A-Za-z0-9](?:[A-Za-z0-9\-]*[A-Za-z0-9])?\.)+(?:lan|local|internal|home\.arpa))"
    r"(?![A-Za-z0-9_\-(\[]|\.[A-Za-z0-9*])")
# Names that end in a private suffix but aren't hosts: the gitignored .env.local file.
HOST_OK = {"env.local"}
# Code, not hosts: self.local, this.internal.
CODE_RECEIVERS = ("self.", "this.")

# Invented test data that is already in the repository's history (the free-text scrubber's
# tests in core/src/text.rs and the forwarded-address tests in server/src/http/guard.rs).
# Nobody's network: listed exactly, so nothing else passes. Don't add to this; use the
# placeholders above in new code.
KNOWN_FIXTURES = {"1.2.3.4", "8.8.8.8", "192.168.250.20", "nas.lan", "printer.local"}


def ip_hits(text):
    out = []
    for m in IPV4.finditer(text):
        try:
            ip = ipaddress.ip_address(".".join(str(int(g)) for g in m.groups()))
        except ValueError:
            continue  # an octet over 255: not an address
        if any(len(g) > 1 and g[0] == "0" for g in m.groups()):
            continue  # leading zeros: a number, not an address
        if any(ip in n for n in ALLOWED_NETS):
            continue
        if VERSION_BEFORE.search(text[max(0, m.start() - 12):m.start()]):
            continue
        if m.group(0) in KNOWN_FIXTURES:
            continue
        out.append(m.group(0))
    return out


def email_hits(text):
    out = []
    for m in EMAIL.finditer(text):
        local, domain = m.group(1).lower(), m.group(2).lower()
        if m.group(0).lower() in EMAIL_NOT_MAIL:
            continue
        if any(domain == d or domain.endswith("." + d) for d in EMAIL_OK_DOMAINS):
            continue
        if local in EMAIL_OK_LOCAL or local.endswith("+noreply"):
            continue
        out.append(m.group(0))
    return out


def host_hits(text):
    out = []
    for m in PRIVATE_HOST.finditer(text):
        name = m.group(1).lower()
        if name in HOST_OK or name in KNOWN_FIXTURES or name.startswith(CODE_RECEIVERS):
            continue
        out.append(m.group(1))
    return out


def generic_hits(text):
    """(kind, match) for every generic finding in a piece of text."""
    return ([("address", h) for h in ip_hits(text)] + [("email", h) for h in email_hits(text)]
            + [("private host name", h) for h in host_hits(text)])


def load_patterns(text):
    """The usable private strings (lower case) and the line numbers of those too short to use."""
    patterns, short = [], []
    for n, line in enumerate(text.splitlines(), 1):
        p = line.strip().lower()
        if not p or p.startswith("#"):
            continue
        if len(p) < MIN_PATTERN:
            short.append(n)
        else:
            patterns.append(p)
    return patterns, short


def pattern_hits(patterns, text):
    low = text.lower()

    def found(p):
        # Plain words ("Tide") match whole words only, so "riptides" or "lockfileVersion"-style
        # identifiers are not hits; anything with digits or punctuation (addresses, ASINs,
        # paths) matches anywhere.
        if re.fullmatch(r"[a-z ]+", p):
            return re.search(r"(?<![a-z0-9])" + re.escape(p) + r"(?![a-z0-9])", low) is not None
        return p in low

    return sorted({p for p in patterns if found(p)})


def git(*args, binary=False, may_fail=False):
    r = subprocess.run(["git", *args], cwd=ROOT, capture_output=True, check=False)
    if r.returncode != 0 and not may_fail:
        sys.exit(f"check-private: git {' '.join(args)} failed: {r.stderr.decode(errors='replace').strip()}")
    return r.stdout if binary else r.stdout.decode("utf-8", errors="replace")


def as_text(data):
    """Bytes as text for scanning. Binary files keep every byte (as Latin-1), so strings inside
    images and other binaries are seen too."""
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError:
        return data.decode("latin-1")


def added_lines(diff):
    """The added lines of a diff (removing a private string must stay possible)."""
    return "\n".join(l[1:] for l in diff.splitlines() if l.startswith("+") and not l.startswith("+++"))


def sources(mode, arg):
    """(label, text) pairs to scan."""
    if mode == "staged":
        names = [n for n in git("diff", "--cached", "--name-only", "--diff-filter=ACMR", "-z").split("\0") if n]
        yield "staged file names", "\n".join(names)
        yield "staged changes", added_lines(git("diff", "--cached", "-U0", "--diff-filter=ACMR"))
        # A binary file's diff is only "Binary files differ": read what is staged instead.
        for line in git("diff", "--cached", "--numstat", "--diff-filter=ACMR", "-z").split("\0"):
            if line.startswith("-\t-\t"):
                name = line[4:]
                yield name, as_text(git("cat-file", "blob", ":" + name, binary=True))
        # (git config fails when a value isn't set: then there is nothing to check.)
        yield "your git name and email", (git("config", "user.name", may_fail=True)
                                          + git("config", "user.email", may_fail=True))
    elif mode == "message":
        yield "the commit message", pathlib.Path(arg).read_text(errors="replace")
    elif mode == "range":
        yield f"commits {arg}", git("log", "-p", "--format=%an %ae %cn %ce%n%B", arg)
    elif mode == "all":
        yield "the whole history", git("log", "--all", "-p", "--format=%an %ae %cn %ce%n%B")
    elif mode == "tree":
        names = [n for n in git("ls-files", "-z").split("\0") if n]
        yield "file names", "\n".join(names)
        for name in names:
            path = ROOT / name
            if path.is_symlink():
                yield name, os.readlink(path)
            elif path.is_file():
                yield name, as_text(path.read_bytes())
    else:
        sys.exit(__doc__)


def patterns_file():
    return pathlib.Path(os.environ.get("ABSHARE_PRIVATE_PATTERNS") or ROOT / "dev" / "local.private-patterns.txt")


def main(argv):
    mode = argv[1] if len(argv) > 1 else "staged"
    if mode == "--self-test":
        return self_test()
    if mode in ("message", "range") and len(argv) < 3:
        sys.exit(__doc__)
    arg = argv[2] if len(argv) > 2 else None
    hook = mode in ("staged", "message", "range")
    err = sys.stderr

    pfile = patterns_file()
    patterns, short = [], []
    if pfile.exists():
        patterns, short = load_patterns(pfile.read_text(errors="replace"))
        if short:
            print(f"check-private: WARNING: {len(short)} private string(s) shorter than {MIN_PATTERN} characters "
                  f"are NOT checked (line(s) {', '.join(map(str, short))} of the list): lengthen or remove them",
                  file=err)
    elif hook:
        print("check-private: " + "!" * 60, file=err)
        print("check-private: WARNING: there is no list of your private strings (dev/local.private-patterns.txt).\n"
              "check-private: Only the generic checks (addresses, emails, private host names) are running:\n"
              "check-private: real titles, names and host names would NOT be caught. Create the list.", file=err)
        print("check-private: " + "!" * 60, file=err)
    else:
        print("check-private: no list of private strings here: running the generic checks only", file=err)

    problems = []
    for label, text in sources(mode, arg):
        lines = None
        for kind, hit in generic_hits(text):
            where = label
            if mode == "tree":
                lines = lines or text.split("\n")
                n = next((i for i, l in enumerate(lines, 1) if hit in l), None)
                where = f"{label}:{n}" if n else label
            problems.append(f"{kind} {hit!r} in {where}")
        for p in pattern_hits(patterns, text):
            problems.append(f"private string {p!r} in {label}")

    if problems:
        print("check-private: refusing, private information found:", file=err)
        seen = sorted(set(problems))
        for p in seen[:50]:
            print(f"  {p}", file=err)
        if len(seen) > 50:
            print(f"  ... and {len(seen) - 50} more", file=err)
        print("check-private: use placeholders (books.example.com, 10.0.0.10, 192.0.2.x, name@example.com)", file=err)
        return 1
    listed = f"{len(patterns)} private strings" if pfile.exists() else "no private list"
    print(f"check-private: clean ({mode}; generic checks and {listed})")
    return 0


def _ip(*octets):
    return ".".join(map(str, octets))


class SelfTest(unittest.TestCase):
    """The examples are put together at run time, so that this file passes its own check."""

    def test_real_looking_addresses_are_found(self):
        for text in (f"server {_ip(192, 168, 1, 20)}:8743;", f'trusted_proxies = ["{_ip(172, 16, 4, 2)}"]',
                     f"ssh {_ip(10, 1, 2, 3)}", f"see http://{_ip(9, 9, 9, 9)}/ for", f"ip={_ip(100, 64, 0, 7)},",
                     f"({_ip(169, 254, 10, 1)})", _ip(10, 0, 1, 10), f"x {_ip(192, 168, 0, 1)}.",
                     f"a\n{_ip(192, 168, 50, 4)}\nb", f"Host: {_ip(10, 89, 1, 1)}", _ip(1, 2, 3, 5)):
            self.assertTrue(ip_hits(text), text)

    def test_placeholders_and_documentation_ranges_pass(self):
        for text in ("10.0.0.10", "10.0.0.255:8743", "127.0.0.1:8743", "listen 0.0.0.0:8743", "192.0.2.7",
                     "198.51.100.200", "203.0.113.1", "255.255.255.0", "10.89.0.20"):
            self.assertEqual(ip_hits(text), [], text)

    def test_numbers_that_are_not_addresses_pass(self):
        v = _ip(4, 3, 2, 1)
        for text in (f"Chrome/{_ip(120, 0, 0, 0)} Safari/537.36", f"version {v}", f"v{v}", f"pkg@{v}", f"{v}.5",
                     "1.2.3", _ip(999, 1, 1, 1), _ip(300, 300, 300, 300), "4.03.2.1", f"x{v}", f"{v}-beta",
                     f"M0.5.{v}"):
            self.assertEqual(ip_hits(text), [], text)

    def test_emails(self):
        at = "@"
        for text in (f"mail jane.doe{at}gmail.com now", f"<someone{at}corp.co.uk>", f"a{at}b.io",
                     f"x+tag{at}proton.me", f"ada{at}example.com.evil.net", f"ada{at}notexample.com"):
            self.assertTrue(email_hits(text), text)
        for text in ("ada@example.com", "ada@mail.example.org", "noreply@anthropic.com", "no-reply@github.com",
                     "1234+user@users.noreply.github.com", "git@github.com:owner/repo.git", "user@host",
                     "@types/node", "a@b", "name@books.example", "x@y.invalid"):
            self.assertEqual(email_hits(text), [], text)

    def test_private_host_names(self):
        dot = "."
        for text in (f"https://store{dot}lan/books", f"ssh media{dot}local", f"books{dot}home.arpa",
                     f"proxy.corp{dot}internal:8443", f"Host: my-server{dot}lan", f"a.b{dot}local.",
                     f"at /mnt/x on box{dot}lan"):
            self.assertTrue(host_hits(text), text)
        for text in (".env.local", "dev/local.env", "dev/local.private-patterns.txt", f"self{dot}local",
                     f"this{dot}internal", f"cfg{dot}internal(", "the plan", "local", "foo.locale", "x.landing",
                     "books.example.com", f"$x{dot}local", "a.localhost", "dev/local.*"):
            self.assertEqual(host_hits(text), [], text)

    def test_private_strings(self):
        host, addr = "store01" + ".lan", _ip(10, 20, 30, 40)
        patterns, short = load_patterns(f"# my list\nSecret Title\n\n{host}\nabc\n  hi  \n{addr}\n")
        self.assertEqual(patterns, ["secret title", host, addr])
        self.assertEqual(short, [5, 6], "short strings are reported by line, not dropped silently")
        self.assertEqual(pattern_hits(patterns, "The SECRET TITLE, again"), ["secret title"])
        self.assertEqual(pattern_hits(patterns, "secret titles"), [], "plain words match whole words only")
        self.assertEqual(pattern_hits(patterns, f"x{host}x 1{addr}1"), sorted([addr, host]))
        self.assertEqual(pattern_hits([], "anything"), [])

    def test_only_added_lines_of_a_diff_count(self):
        diff = f"--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old {_ip(192, 168, 1, 1)}\n+new 10.0.0.10\n"
        self.assertEqual(added_lines(diff), "new 10.0.0.10")
        self.assertEqual(generic_hits(added_lines(diff)), [])
        self.assertTrue(generic_hits(added_lines(diff.replace("-old", "+old"))))

    def test_binary_bytes_are_scanned(self):
        text = f"made on store{'.'}lan by jane{'@'}gmail.com at {_ip(192, 168, 7, 7)}"
        blob = b"\x89PNG\r\n\x00\xff\xfeComment\x00" + text.encode() + b"\x00\xff"
        kinds = sorted(k for k, _ in generic_hits(as_text(blob)))
        self.assertEqual(kinds, ["address", "email", "private host name"])

    def test_the_known_fixtures_are_exact(self):
        for known in KNOWN_FIXTURES:
            self.assertEqual(generic_hits(f"at {known} today"), [], known)
        self.assertTrue(generic_hits("at my" + "nas" + ".lan today"))
        self.assertTrue(generic_hits("at " + _ip(192, 168, 250, 21)))


class HookTest(unittest.TestCase):
    """The script and the tracked hooks, run for real in a throwaway repository."""

    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp(prefix="check-private-"))
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        (self.tmp / "tools").mkdir()
        shutil.copy(pathlib.Path(__file__).resolve(), self.tmp / "tools" / "check-private.py")
        shutil.copytree(ROOT / ".githooks", self.tmp / ".githooks")
        self.env = {"PATH": os.environ.get("PATH", ""), "HOME": str(self.tmp), "GIT_CONFIG_NOSYSTEM": "1"}
        for args in (["init", "-q"], ["config", "user.name", "Ada Example"],
                     ["config", "user.email", "ada@example.com"], ["config", "core.hooksPath", ".githooks"]):
            self.git(*args)

    def git(self, *args, env=None):
        return subprocess.run(["git", *args], cwd=self.tmp, env={**self.env, **(env or {})},
                              capture_output=True, text=True, check=False)

    def check(self, *args, env=None):
        return subprocess.run([sys.executable, "-I", str(self.tmp / "tools" / "check-private.py"), *args],
                              cwd=self.tmp, env={**self.env, **(env or {})}, capture_output=True, text=True,
                              check=False)

    def write(self, name, data):
        path = self.tmp / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data if isinstance(data, bytes) else data.encode())
        self.git("add", name)

    def test_the_hooks_are_executable(self):
        for hook in ("pre-commit", "commit-msg", "pre-push"):
            self.assertTrue(os.access(ROOT / ".githooks" / hook, os.X_OK), hook)

    def test_a_clean_commit_passes_and_the_missing_list_is_announced_loudly(self):
        self.write("notes.txt", "listen on 10.0.0.10 for books.example.com\n")
        r = self.git("commit", "-q", "-m", "Add notes")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("WARNING: there is no list of your private strings", r.stderr)
        self.assertIn("!!!!!!!!", r.stderr)
        r = self.check("tree")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("running the generic checks only", r.stderr)
        self.assertNotIn("!!!!", r.stderr, "quiet about it outside the hooks (CI has no list)")

    def test_a_staged_address_email_or_host_stops_the_commit(self):
        for n, bad in enumerate((_ip(192, 168, 4, 9), "jane" + "@" + "gmail.com", "store" + ".lan")):
            self.write(f"file{n}.txt", f"see {bad}\n")
            r = self.git("commit", "-q", "-m", "Add a file")
            self.assertNotEqual(r.returncode, 0, bad)
            self.assertIn(bad, r.stderr)
            self.git("rm", "-q", "--cached", f"file{n}.txt")

    def test_a_staged_binary_is_read(self):
        self.write("cover.png", b"\x89PNG\r\n\x1a\n\x00\x00\xff" + ("store" + ".lan").encode() + b"\x00\xfe\xff")
        r = self.git("commit", "-q", "-m", "Add a cover")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("cover.png", r.stderr)

    def test_the_commit_message_is_checked(self):
        self.write("ok.txt", "fine\n")
        r = self.git("commit", "-q", "-m", "Deployed to " + _ip(172, 16, 9, 9))
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("the commit message", r.stderr)

    def test_removing_a_private_string_is_allowed_but_history_is_still_checked(self):
        bad = _ip(192, 168, 4, 9)
        self.write("old.txt", f"see {bad}\n")
        self.assertEqual(self.git("commit", "-q", "--no-verify", "-m", "Slipped through").returncode, 0)
        self.write("old.txt", "see 10.0.0.10\n")
        r = self.git("commit", "-q", "-m", "Use a placeholder")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.check("tree").returncode, 0)
        for history in (["all"], ["range", "HEAD"]):
            r = self.check(*history)
            self.assertEqual(r.returncode, 1, history)
            self.assertIn(bad, r.stderr)

    def test_the_private_list_is_used_and_short_entries_are_reported(self):
        plist = self.tmp / "private.txt"
        plist.write_text("Secret Title\nabc\n")
        env = {"ABSHARE_PRIVATE_PATTERNS": str(plist)}
        self.write("book.txt", "The Secret Title, unabridged\n")
        r = self.check("staged", env=env)
        self.assertEqual(r.returncode, 1)
        self.assertIn("secret title", r.stderr)
        self.assertIn("1 private string(s) shorter than 5 characters are NOT checked (line(s) 2", r.stderr)
        self.assertNotIn("abc", r.stderr, "the short string itself isn't printed")
        self.assertNotIn("no list", r.stderr)
        r = self.check("tree", env=env)
        self.assertEqual(r.returncode, 1, "tracked files are checked against the list too")


def self_test():
    load = unittest.defaultTestLoader.loadTestsFromTestCase
    suite = unittest.TestSuite([load(SelfTest), load(HookTest)])
    return 0 if unittest.TextTestRunner(verbosity=1).run(suite).wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
