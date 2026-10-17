# Features

Everything ABShare does today, by area. Only built and tested features are listed; planned work is in [Not yet](#not-yet) at the end.

## Highlights

- An installable web app for iPhone, iPad, Android and desktop that keeps playing with the screen locked.
- Your place in every book follows you between devices, including listening done offline.
- Data saver for mobile networks, with exact seeking.
- Downloads for offline listening, kept separately for each person.
- Invite-only accounts with passkeys or a password and authenticator app.
- A read-only library, sandboxed ffmpeg and no outbound network connections.

## Listening

| Feature | Details |
|---|---|
| Player | Full-screen player and a mini player on every screen. Large cover, book and chapter title. |
| Scrubber | Chapter-relative, with whole-book progress and time left in the book underneath. The scrubber seeks when you let go, not while dragging. |
| Skip | Back and forward, each 5, 10, 15, 30, 45 or 60 seconds (15 back and 30 forward by default). |
| Chapters | A chapter list, and previous and next chapter buttons. Books with no chapters but several parts get one chapter per part. |
| Speed | 0.75× to 3× (0.75, 0.9, 1, 1.1, 1.2, 1.3, 1.5, 1.75, 2, 2.5, 3), with the pitch kept. Remembered for each book, with a default of your choosing. |
| Sleep timer | 5, 10, 15, 30, 45 or 60 minutes, or the end of the chapter. Fades out over the last 10 seconds. Works in the background. |
| Auto-rewind | After a pause, playback steps back a little: nothing under a minute, 2 seconds under 10 minutes, 5 seconds under an hour, 10 seconds after that. |
| Bookmarks | Add a bookmark at the current position from the player, see a book's bookmarks and jump to them, and delete them. |
| Lock screen and headsets | Title, chapter, author and cover on the lock screen; play, pause, skip, seek, and previous and next chapter from the lock screen, headsets and car controls (Media Session). AirPlay through Safari's own controls. |
| Mark finished | Books are marked finished near the end automatically. You can also mark a book finished or unfinished, or start it again. |
| Restore | Reopening the app restores the last book and position, ready to play. |

## Your place, everywhere

| Feature | Details |
|---|---|
| Progress sync | Your position is saved every 15 seconds while playing, on pause and seek, and when the app goes to the background. |
| Across devices | When another device has listened further, a paused player moves to the newer position. |
| Offline | Positions and finished marks made offline are queued on the device and sent when it is back online. The newest listening wins, and a repeated or out-of-date update can't move you backwards. |

## Streaming quality

| Feature | Details |
|---|---|
| Original | The book as it is, or a playable copy for multi-part books and formats browsers can't play. |
| Data saver | A mono MP3 at 48 kb/s (about 21.6 MB an hour), or 40 or 32 kb/s if you prefer. Seeking, the lock screen and your position work exactly as with the original. |
| Automatic | Uses data saver when the browser reports a cellular or data-saving connection (Chrome on Android reports this; Safari and Firefox don't, so they get Original unless you choose). |
| Switch while listening | Change quality from the player's menu; playback carries on at the same place. |

## Offline

| Feature | Details |
|---|---|
| Downloads | Download any book in original quality or data saver. The size and the space available are shown first. |
| Resumable | Books download in 4 MiB pieces and carry on from where they stopped if interrupted. Keep ABShare open while it downloads. |
| Offline playback | A downloaded book always plays from the device, with seeking, even with no connection at all. |
| Downloads page | Each book's state and size, total storage used against what the browser allows, and remove buttons. |
| Persistent storage | ABShare asks the browser to keep downloads. On iPhone and iPad, add ABShare to the Home Screen so Safari keeps them. |
| Per person | Each person's downloads, queued progress and speeds are kept separately. Signing out offers to keep or remove them. A device signed out remotely wipes its downloads. |
| Save file | People with the *save files* permission can save a book's audio file to their device. |

## Browsing

| Feature | Details |
|---|---|
| Home | Continue listening (with time left), the next book in each series you are reading, recently added, and books on this device. |
| Library | All your books and shows, with search, a status filter (not started, in progress, finished) and sorting by title, author, recently added or recently played. |
| Search | Titles, authors, narrators, series and genres, ignoring case and accents. |
| Genres | Store genre tags are sorted onto 17 shelves. Fiction: Science Fiction, Fantasy, Mystery & Thriller, Horror, Action & Adventure, Literature & Classics, Comedy, Romance, Children & Young Adult. Non-fiction: Biography & Memoir, History, Science & Technology, Politics & Society, Philosophy & Religion, Business, Money & Self-help, Arts, Entertainment & Travel. Plus Podcasts. A book can sit on several shelves. |
| Series | Books sharing a series are grouped automatically, in reading order (prequels first), with a "Book 2 of …" link on each book. |
| Shows | Podcast-style episodes are recognised and shown as one show, ordered by season and episode, with trailers and extras set aside. |
| Book page | Cover, title, series, authors, narrators, genres, length, time left, description and chapters. Download, mark finished or unfinished, start again, bookmarks and save file. |
| Covers | Embedded or folder artwork, resized for each screen. |
| Themes | Material (from your choice of 8 accent colours), Classic and High contrast; light, dark or following the system. |

## Accounts and security

| Feature | Details |
|---|---|
| Invites | Single-use invite links that expire (72 hours by default, up to 7 days). No open sign-up. |
| Passkeys | Sign in with Face ID, a fingerprint or a PIN. Add several and remove any. |
| Password and authenticator | A password (at least 12 characters, checked against a list of breached passwords) always needs a code from an authenticator app. |
| Recovery codes | Ten single-use codes at enrolment, renewable. Signing in with one takes you straight to set up a new passkey or authenticator app; the rest of the app opens once you have. |
| Devices | See where you are signed in, and sign any device out. Changing your password, or removing a passkey or authenticator, signs out your other devices. |
| Step-up | Changing your sign-in methods asks you to confirm it's you again. |
| Lockouts | Repeated failures lock the address out, with doubling delays. A stranger can't lock you out of your own account. |
| Remote sign-out | A device signed out from elsewhere stops working at once, and wipes its downloads. |

## Administration

| Feature | Details |
|---|---|
| Overview | People, admins, open invites, two-step coverage, the ffmpeg sandbox state, cache size and version, with warnings (for example, no trusted proxy, or only one admin). |
| People | Change display names, roles, library access and permissions (offline, save files); disable accounts; reset someone's sign-in methods (with a new single-use link); sign all their devices out. Making someone an admin needs a second confirmation. |
| Invites | Create invites with a name, role, libraries, permissions and expiry; copy the link; revoke unused ones. |
| Library | The last scan, *Scan now*, each library's counts, and problems: unplayable files, unsupported formats, ignored duplicates, and missing books. |
| Edit details | Correct any book's title, people, series, genres or description. Corrections survive rescans. |
| Audit log | Sign-ins, failures, locks, enrolment, factor changes, invites, device sign-outs and admin actions, with a check that no entry has been edited or removed. |
| Step-up | Every admin change asks for a fresh second factor. |
| Command line | `abshare admin invite`, `reset`, `users`, `devices`, `revoke-all` and `verify-audit`, for bootstrap and recovery. `abshare doctor` checks the whole installation. |

## The library

| Feature | Details |
|---|---|
| Formats | `.m4b`, `.m4a`, `.mp4`, `.mp3`, `.ogg`, `.oga`, `.opus`, `.flac` and `.wav`. DRM files (`.aax`, `.aa`) and xHE-AAC are listed as unsupported. |
| Layouts | A folder per book, author and series folders, numbered parts in subfolders, `CD1`/`CD2` discs, flat folders of many books, and loose files. |
| Duplicates | Copies of the same book are recognised and the best one is used. |
| Metadata | Embedded tags first, then sidecar text files and folder names. Store conventions are understood: narrators in `composer`, "(Unabridged)" and "Book N" suffixes, sentinel part numbers, HTML descriptions. |
| Rescans | Scheduled (hourly in the examples) or on demand. Only new or changed files are examined; missing books are kept, not deleted. |
| Read-only | ABShare never writes to the library, and refuses to start if it could. |
| Several libraries | Up to 32, each with its own people. |

## Not yet

These are designed in [`SPEC.md`](../SPEC.md) but not built:

- Listening history and statistics.
- Author, narrator and series filters in the Library, and author pages.
- Editing a bookmark's note in the app (the API supports it).
- A "Downloads" settings page (default quality, Wi-Fi only).
- Admin pages for the job queue and server settings (settings live in the config file).
- Native apps.
