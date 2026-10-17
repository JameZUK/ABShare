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
- **The audit log:** security events (sign-ins, failed attempts, account changes, admin
  actions) with time, account, device and IP address. Kept for at most a year.

Admins can see the account list, devices, permissions and the audit log. They can't see your
password, authenticator secret or recovery codes.

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
