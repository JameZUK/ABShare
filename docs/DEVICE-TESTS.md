# Device tests

The checks only a real phone can do. They are the device gates of SPEC §11 (Phases 3 and 4), plus the features since 0.10 that depend on the device: the lock screen, the screen being off, airplane mode and killing the app. The browser tests in CI cover everything else.

**You need:**
- an iPhone with ABShare installed to the Home Screen;
- a second device signed in as the same person (another phone, an iPad or a computer);
- a book that is downloaded and one that isn't.

Allow about 15 minutes of tapping, plus the long-running tests (2 hours locked, 1 hour in airplane mode), which can run while you do something else.

Record the device, OS version and ABShare version (Settings → About) with the results.

## 1. Lock screen and background (Phase 3 gate)

- [ ] **Two hours locked.** Start a book, lock the phone and leave it. After 2 hours it is still playing, and the position has moved on by about 2 hours.
- [ ] **Lock-screen controls.** The lock screen shows the cover, the book and chapter title, and play/pause. Skip back and forward move by your skip intervals (iOS may show previous and next chapter instead), and dragging the lock-screen position seeks.
- [ ] **Headphones.** Pause and play from the headphones (or AirPods). Taking them out pauses playback.
- [ ] **Interruptions.** A phone call or a Siri request pauses the book; afterwards it can be resumed from the lock screen.
- [ ] **Sleep timer.** Set 5 minutes, lock the phone. It fades out and stops after 5 minutes.

## 2. Two devices (Phase 3 gate)

- [ ] **Resume elsewhere.** Listen on the phone for a minute, then pause. Open the same book on the second device within 15 seconds: it starts where the phone stopped.
- [ ] **Back again.** Listen on the second device, pause, then return to the phone (with the player paused). The phone moves to the newer position by itself.

## 3. Offline (Phase 4 gate)

- [ ] **An hour in airplane mode.** Turn on airplane mode, play a downloaded book for an hour, then turn airplane mode off and open the app. Within a minute, the second device shows the new position.
- [ ] **Offline listening reaches history.** After that hour: Settings → History counts the hour, and "Where was I?" for the book shows the stretch heard offline.
- [ ] **A big download survives the app being killed.** Start downloading a large book (2 GB or more), swipe the app away part-way, reopen it. The download carries on from where it was, not from the start.
- [ ] **A slow connection.** On a weak mobile signal (or with a slow network set in the phone's developer options), close the app fully and reopen it. Within a few seconds the app opens with "Connecting… The connection is slow." A downloaded book plays straight away. When the connection catches up, the banner goes and Home fills in. It never says "Offline" unless the connection is really lost.
- [ ] **Not downloaded, no connection.** In airplane mode, opening a book that isn't downloaded says it needs a connection, and nothing breaks.
- [ ] **Wi-Fi only.** Settings → Downloads → *Download only on Wi-Fi*. On iPhone the page says the browser doesn't say whether you're on Wi-Fi, so downloads aren't held. (On Android Chrome: with mobile data, a new download waits with "Waiting for Wi-Fi".)

## 4. The player

- [ ] **The slider.** Drag the scrubber in the full player: the time label follows your finger, and playback jumps to where you let go. Small adjustments are possible with one thumb.
- [ ] **Scrubbing fast doesn't trip the limit.** In data saver, drag the native scrubber back and forth quickly for a few seconds (Firefox on Android fires many seeks), then let go. Playback starts at the new place within a few seconds, with no "couldn't be played" message. (Each person may have 6 audio requests running at once; abandoned ones should free their place straight away.)
- [ ] **The mini player.** It shows on every screen while a book is open; tapping it opens the full player, and play/pause works from it.
- [ ] **Speed.** Change speed in the player; it stays for that book after closing and reopening the app.
- [ ] **Data saver.** Settings → Playback → *Data saver*. Playback continues, and seeking anywhere in a long book starts within a second or two.

## 5. Features since 0.10

- [ ] **"Where was I?"** Listen in two stretches with a seek between them. The book's menu → *Where was I?* lists both. Tapping a start or end point plays from there.
- [ ] **"How was it?"** Finish a book (or mark it finished). A rating sheet appears once. It doesn't appear again for that book.
- [ ] **Reviews.** Write a review on a book page: the stars are easy to tap, and the word counter and keyboard don't hide the Save button.
- [ ] **Recommendations.** As an admin, recommend a book to yourself. It appears on Home; *Not now* removes it.
- [ ] **Paperback, dark.** Switch the phone to dark mode: the app follows, and text stays easy to read in a dark room.

## 6. Signing out

- [ ] **Remove downloads.** Settings → Account → Sign out → *Remove downloads*. Signing back in, Settings → Downloads shows nothing stored.
- [ ] **Remote sign-out.** On the second device, sign the phone out (Settings → Devices). When the phone next opens the app, it is signed out and its downloads are gone.

## Results

| Date | Device and OS | ABShare | Passed | Notes |
|---|---|---|---|---|
| | | | | |
