# Data compiled into `core`

## `common-passwords.bin`

Passwords of 12 or more characters from a public list of the million most common leaked
passwords, used to refuse them at enrolment and password change (SPEC §5.5, ASVS V6).
Each entry is lowercased (printable ASCII only), hashed with SHA-256, and stored as the first
8 bytes of the hash; the file is those 8-byte prefixes, sorted, with no separators. Shorter
entries are left out because ABShare refuses passwords under 12 characters anyway. Hashes
keep the words themselves out of the repository; a false match needs a 64-bit collision.

- Source: SecLists, `Passwords/Common-Credentials/xato-net-10-million-passwords-1000000.txt`
  at commit `c205c36a445bff37f8e58a9ec829105cd4975c58`
  (SHA-256 of the source file
  `424a3e03a17df0a2bc2b3ca749d81b04e79d59cb7aeec8876a5a3f308d0caf51`).
- Licence: MIT, Copyright (c) 2018 Daniel Miessler.

To rebuild: take the source file, keep printable-ASCII lines of 12 to 64 characters, lowercase
them, de-duplicate, and write the sorted first 8 bytes of each SHA-256.
