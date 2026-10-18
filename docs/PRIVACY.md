# Privacy

ABShare is software you run yourself. Whoever runs a server controls its data; nothing is
sent to the authors of ABShare or anyone else. The server makes no outbound connections, and
the app loads nothing from other websites.

## What the server stores

- **Your account:** username, display name, role and permissions; your passkeys' public keys;
  your password as an argon2id hash; your authenticator secret, encrypted; your recovery
  codes as hashes.
- **Your devices:** a name you chose (such as "iPhone · Safari"), when it signed in and was
  last seen, its last IP address and browser user agent. These are cleared when the device
  is signed out.
- **Listening:** for each book you've played, your position, whether you finished it and
  when that changed; your bookmarks and their notes.
- **Listening history:** sessions of listening built from the positions your app reports:
  the book, the device, when each session started and ended, and how long you listened. They
  give your listening time (last 7 days, last 30 days, all time) in Settings → History.
  Sessions are kept for 13 months, and at most 50,000 per person (the oldest go first). You
  can clear them all at any time, or turn history off so nothing new is recorded; your place
  in each book is kept either way. Listening while offline isn't counted.
- **The audit log:** security events (sign-ins, failed attempts, account changes, admin
  actions) with time, account, device and IP address. Kept for at most a year.
- **Start-up reports:** if the app is left showing a blank page (or the address has
  `?diag`), the browser sends a short text report: the error, browser user agent, screen
  size, colours and whether cookies are allowed, with the IP address it came from. Reports are
  kept in memory only (the newest 50, gone when the server restarts) and written to the
  server log. No account is involved.

Admins can see the account list, devices, permissions and the audit log. They can also see
each person's listening: time heard (last 7 and 30 days and in all), when they last listened,
which books they have finished, are listening to or have abandoned (started, not finished,
nothing heard for 30 days), time heard per book, and recent sessions (which book, when, and
for how long). They can't see your password, authenticator secret or recovery codes, or where
you are in a book: positions, and the "Where was I?" timeline of where each stretch of
listening began and ended, are only ever shown to you. Clearing or turning off your history
isn't written to the audit log. (Whoever runs the server can still read its database directly.)

- **Reviews:** a star rating and an optional comment of up to 50 words, with when it was
  written. Everyone who can see the book sees your review with your display name. You can
  change or delete it at any time; admins can remove it.
- **Recommendations:** what an admin recommended to whom, with their note, and which ones you
  dismissed.

## What your device stores

- In the browser's local storage: your app preferences (playback, appearance), the last book
  you opened, and the name of the account last used on this device.
- In IndexedDB, per account: downloaded books, their covers and details, your positions,
  and progress waiting to be sent to the server.
- Cookies: a refresh token (for signing in again without your passkey), and a short-lived
  media token for playing audio. Both are HttpOnly and sent only to this server.

Signing out asks whether to keep or remove downloads on the device. If an admin or you sign
the device out remotely, its downloads are removed the next time the app opens.

## Deleting your data

Ask the person who runs your server to delete your account. They can disable it at once from
the admin screens.
