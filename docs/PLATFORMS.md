# Platforms and status

## Where it runs

ABShare is a web app: there is nothing to install from a store. Open your invite link in the browser, and add it to your Home Screen (iPhone and iPad) or install it (Chrome, Edge) to use it like an app.

| Platform | Browser | Notes |
|---|---|---|
| iPhone and iPad | Safari, iOS and iPadOS 17.1 or later | Add ABShare to the Home Screen: Safari keeps downloads only for Home Screen apps, and that is how the lock screen and background playback work best. The volume control is hidden because iOS doesn't allow web pages to set it. |
| Android | Chrome, Firefox | Chrome reports mobile data, so Automatic quality switches to data saver on its own. Install from Chrome's menu for an app icon. |
| Windows, macOS, Linux | Chrome, Edge, Firefox, Safari | Everything works, including downloads. |

Passkeys need a browser and device that support them (all current ones do). Otherwise use a password with an authenticator app.

**Tested on real devices so far:** iPad Safari (playback with the screen off, seeking, offline playback from downloads), Firefox for Android (background playback, seeking, offline playback, streaming on mobile data), and desktop Chromium. The test suite runs the app's logic in a browser-like environment on every push.

## The server

Linux only, because the ffmpeg sandbox needs Landlock (kernel 6.12 or later, enabled in the kernel's LSM list). Supported deployments are the Docker image, the systemd units, and the Docker image inside an unprivileged LXC container. See [`DEPLOY.md`](DEPLOY.md).

## Project status

| Phase | State |
|---|---|
| 0: Spikes (offline playback, background audio, ffmpeg, the jail, passkeys, data saver) | Done |
| 1: Core and scanner | Done |
| 2: Accounts and sign-in | Done |
| 3: Streaming and the app | Built; device tests in progress |
| 4: Offline | Built; device tests pending |
| 5: Admin and deployment | Done |
| 6: Security review and release | AI-assisted review and penetration test done, with no open critical, high or medium findings; an independent review is recommended |

The device tests still to run include a 2-hour locked-screen session on a phone, lock-screen controls on iPhone, resuming across two devices, an hour of airplane-mode listening followed by a sync, and resuming a large download after the app is killed.

## Roadmap

Planned, in no fixed order:

- Author, narrator and series filters, and author pages.
- Editing bookmark notes in the app.
- An independent security review.

Native apps are not planned until the web app has proved itself. The API is versioned and documented ([`API.md`](API.md)) so they can come later.
