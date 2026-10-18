# Security review of the 0.10 features (for 0.11.0)

Done 2026-10-18. It covers everything added after the Phase 6 review (0.9.0):
- reviews and recommendations;
- "Where was I?" and offline listening in history;
- the admin listening view;
- the "How was it?" prompt;
- Downloads settings;
- the Paperback redesign, with its fonts and CSP change;
- the new Admin → Server settings page.

It was done in three parts:
- **Server code review:** the routes, migration 9 and the `core` rules they use (findings S1–S11).
- **Web app code review:** every changed screen and store (findings W10–W17).
- **Black-box test:** a throwaway local server on the invented demo library, with an admin and a listener (findings P10 on).

**This is not the independent review SPEC §9.4 asks for.** Like Phase 6, it was AI-assisted. An independent human review is still recommended before public exposure (D22).

**Severity** reflects realistic exploitability for an invite-only service on the internet. Every fix has a regression test, and each was shown to fail before the fix. The exceptions are S5, which adds a missing test for a control that already worked, and W16 and W17's off-by-one, both noted in their rows.

## Outcome

- **No Critical or High findings.**
- **The one Medium (S1) and every Low are fixed,** each with a regression test.
- **Info findings are fixed, or accepted with a reason** (S8).

**Status:** **Fixed** means fixed with a test. **Accepted** is explained in the row.

## Medium

| ID | Finding | Status |
|---|---|---|
| S1 | **Offline replay could hold the database for everyone.** A replay of 200 reports that each start a new stretch ran the 50,000-session cap 200 times inside one transaction on the server's only database connection (about 0.4 s at the cap). The limit was 12 a minute *per device*, and a person can have 20 devices. | **Fixed:** the limit is 12 a minute per person, across all their devices; the cap runs once per request, not once per new stretch; test (two devices share the allowance). |

## Low

| ID | Finding | Status |
|---|---|---|
| S2 | **Replay could count the same listening again.** Only an exact retry (same key) was caught. The same reports under a new key started an overlapping stretch, so time heard (in history and the admin view) could grow without limit. That contradicted D33's claim. | **Fixed:** reports at or before the newest moment that device has recorded for the book are skipped; test (the same batch under three keys counts once). |
| S3 | **A comment one word over the limit could be silently cut.** If the character after the 400th was a space, cleaning cut and trimmed there, and the shortened text passed. That broke D28's "refused, never cut" rule; recommendation notes were affected too. | **Fixed:** text is cleaned with room to spare before it is counted; core test; the fuzz target now asserts that accepted text is never shortened. |
| S4 | **Privacy wording.** D26 said clearing history isn't audited "so admins don't learn of it". But the admin view shows whether history is on, and the books list (started, finished, last played) comes from progress, which continues with history off. | **Fixed (docs):** this matches the decision that admins see all listening. PRIVACY.md and D26 now say exactly what admins see. |
| S5 | **No test that withdrawing a recommendation needs step-up.** The control was there; removing it would have failed no test. | **Fixed:** test covering all three new admin writes without step-up. |
| W10 | **The "How was it?" memory survived *Remove downloads* and revocation.** It is a per-person list of books they finished, in local storage. | **Fixed:** removed with the person's other data on this device, including an interrupted wipe finished at the next start; test. |
| W11 | **A replay retried after a lost answer used a new key.** Together with S2, the same listening was counted twice. | **Fixed:** a batch's key is derived from its first item, so it's the same on every attempt; test. (S2 also makes this harmless on the server.) |
| W12 | **The start-up retry after a 429 dropped other answers.** A revocation during the wait didn't remove the downloads, and an outage showed the sign-in screen instead of going offline. The wait also used the first answer's `Retry-After`. | **Fixed:** every answer, including retries, goes through the same handling; tests for both cases. |
| W13 | **Click-to-resume didn't clamp the start.** Tapping the end of a stretch that ran to the end of the book opened the book at its end, which finished it at once and asked "How was it?". | **Fixed:** start positions are clamped to the book and stop short of its end; test. |
| W14 | **Stale reviews after moving between books.** A slow answer for the previous book could fill the next book's reviews. An admin's *Remove* could then target the wrong book, and saving could copy the old rating and text over. | **Fixed:** answers for another book are ignored, and the component is recreated for each book; component test. |

## Info

| ID | Finding | Status |
|---|---|---|
| S6 | Replay keys were never pruned (only progress pruned the shared key table). | **Fixed:** replay prunes keys older than 100 days, like progress; test. |
| S7 | The review and dismissal routes missed the 16 KiB body limit: they were merged after the library routes' limit was applied, so 64 KiB applied. | **Fixed:** the limit is set on those routes; test (20 KiB → 413). |
| S8 | `recommendation.by_user` has no `ON DELETE` rule. | **Accepted:** no route or command deletes users, since accounts are disabled instead. Changing a foreign key in SQLite means rebuilding the table, which isn't worth doing for a path that doesn't exist. Whoever adds user deletion must handle recommendations; this row is the reminder. |
| S9 | Replay with history off answered `"recorded": n`. | **Fixed:** it counts only what changed the history (0 with history off); test. |
| S10 | Dismissing an "everyone" recommendation of only books the caller can't see answered 204, revealing that it exists. | **Fixed:** 404, as for one not addressed to you; test. |
| S11 | No rate limit on review writes or dismissals. | **Fixed:** 20 a minute per device; test. |
| W15 | Two reviews with the same name and time had the same list key, which stopped the reviews from rendering. | **Fixed:** keyed by position; component test. |
| W16 | The HTML-injection scan didn't check inline `style` attributes built from data, or `action`, `formaction`, `poster` and `xlink:href`. No current code was affected. | **Fixed:** inline styles built from data are banned (`style:` directives only), and those attributes join the safe-prefix check; the scan tests itself on each form. |
| W17 | While listening, each new position sent the outbox at once, ignoring a `Retry-After` wait. Separately, the heard-report trim kept one fewer than the limit. | **Fixed:** while a retry is scheduled, new positions wait for it; test. The trim is corrected. It isn't separately tested, because that would need 10,000 stored reports. |

## Black-box test

A local debug build ran on the invented demo library, plus a second library made from another demo build. It had an admin and three listeners, one of them limited to one library. About 180 requests in 35 groups of checks.

**What held:**
- **Authorisation:** listeners get `403` on every new admin route, and `401` with no token or a bad one.
- **Ownership:** no one can dismiss or delete what isn't theirs.
- **Privacy:** each timeline is visible only to its owner. Admins can't reach any per-book timeline (8 path and parameter variants tried), and no position appears anywhere in the admin listening data.
- **Library boundaries:** books in other libraries answer `404` on every route, and recommendations leave them out.
- **Validation:** strict on every body (stars, unknown fields, types, numbers at the extremes, the word and character limits, note and target limits), and on replay keys, counts, times and positions.
- **Cleaning:** HTML, bidirectional overrides, invisible characters and controls are removed from comments and names, and marks are capped.
- **Admin writes:** step-up and audit on every one.
- **Protocol:** content types and methods are checked, and the security headers appear on every answer, errors included.
- **Accounts:** the factor gate after a recovery sign-in applies to the new routes, and a disabled person's reviews are hidden.
- **Stability:** no panics.

Six findings matched the code review's: P10 is S1, P11 is S2, P12 is S11, P13 is S7, P14 is S4 and P16 is S9. P20 is the targeted form of S10, and the same fix covers it.

| ID | Finding | Status |
|---|---|---|
| P15 | Info. Dismissing a recommendation ignored any body, even 60 KB of text, although it takes none. | **Fixed:** a body is refused (`400`); test. |
| P17 | Info. Unknown query parameters were ignored on the new GET routes, although API.md says they're refused. | **Fixed:** the reviews, timeline, admin listening, recommendations and settings routes refuse them (`400`); test. |
| P18 | Info. People who lost access to a book could no longer delete their review of it, which others could still read. | **Fixed:** your own review can be removed whatever your current access; it touches only your own row; test. |
| P19 | Info. An admin limited to some libraries still sees and can recommend books outside them through the admin routes. | **Accepted:** admins can lift their own limits (with step-up), so a limit on an admin is a filter for their own listening, not a security boundary. |
| P21 | Info. Recommendations addressed to a disabled person stay, and show again if the account is re-enabled. | **Accepted:** by design. New ones can't target a disabled person. |

**Outside this surface:** a recovery-code sign-in marks the whole account as needing a new factor, so the person's other signed-in devices get `403 factor_required` until they set one up. That is intended (review Phase 6, recovery re-enrolment) and documented in API.md.
