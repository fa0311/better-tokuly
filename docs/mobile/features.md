# Mobile: what each area does

What the phone app does natively per area, what still opens on the web, and
where the code is. The rules a contributor follows are in
[`apps/mobile/README.md`](../../apps/mobile/README.md); paths are under
`apps/mobile/src/` unless they say otherwise.

| Area | Native | On the web for now | Code |
|---|---|---|---|
| Sessions | sign-in, several accounts, switching, sign-out | — | `features/session` (`vault.ts`, `store.ts`, `session.ts`) |
| Location | sharing on screen at the server's cadence; in the background with "always" allowed (50 m filter, paused while still) | — | `features/map/sharing.ts`, `@tokuly/core/map` |
| Chat | conversations, timelines, unread totals, typing, read cursors, photos, an outbox (in order, idempotency key, retried by hand) | calls, the AI assistant | `features/chat`, `@tokuly/core/chat` |
| Camera and uploads | photo and video (VisionCamera), trimming on the device (`modules/tokuly-trim`, no FFmpeg), resumable uploads | — | `features/media` (`camera-screen.tsx`, `posts.ts`, `upload.ts`) |
| Stories | tray, viewer, reactions, replies (as DMs), who watched, friends or chosen close friends | — | `features/stories`, `@tokuly/core/stories` |
| Shorts | the three feeds, likes, comments, sharing, hashtag and sound pages, posting with every audience, the creator's list | — | `features/shorts`, `@tokuly/core/shorts` |
| Videos | feeds, watch screen (HLS, picture-in-picture, lock-screen controls, resume, chapters), playlists, subscriptions, comments, channel pages, history | uploading, editing, analytics, watch parties | `features/video`, `@tokuly/core/video` |
| Notes | the timelines, lists and antennas to read, rich text, polls, likes, renotes, quotes, replies, reactions, clips, pinning, thread mutes, composer with drafts | channels, editing a note, managing lists and antennas, mutes, analytics, reports | `features/notes`, `@tokuly/core/notes` |
| Live | lists, watching (Low-Latency HLS), chat, likes, reminders, polls, reports, gifts | going live (no WHIP/RTMP publisher in the app), clips, gift history, buying coins, moderation | `features/live`, `@tokuly/core/live` |
| Search | every kind by tab, typeahead, trends, recent searches (forget, clear, pause); a hit opens its screen or its web screen | sorting, searching within one profile | `features/search`, `@tokuly/core/search` |
| Push | FCM and APNs through `expo-notifications` (no Expo push service); a tap opens the notification's screen | — | `features/notifications` (`push.ts`, `target.ts`) |
| Everything else | — | spaces, communities, school, plans, ask, quests, emoji, stamps, wallet, most settings, developers, the console | `features/web` |

## Rules that hold everywhere

- **Switching accounts starts the signed-in tree over** (API object,
  realtime connection, query cache, navigation stack), like a page load on
  the web.
- **Everything belongs to the account that started it.** An upload, a post
  in progress, a playback or view report is only ever sent as that account,
  stops when another takes over (the feature's `lifecycle.ts`) and is thrown
  away when the account leaves the device.
- **Location and push are the account's decision on this device.** Nothing
  is sent, and no push token registered, until the account says yes here.
  A stop, a sign-out or a switch tells the server first, written down as
  owed before it is sent; what did not reach it is retried at every start
  (a location stop with its own time, void once the account shares again),
  so the next account never gets the previous one's pushes.
- **Tokens**: the access token in memory, refresh tokens in secure storage.
  A signed-out session's token is kept until the server has ended the
  session and removed its push registrations.
  Once an account left, its API never refreshes (`lib/api.ts`): only a call
  with the access token it still holds goes out, any other is refused
  (`ACCOUNT_LEFT`). A sign-out waits behind a switch on its way and ends the
  account that was in use when it was asked.
- **Limits come from the server**: the longest recording per purpose
  (stories 60 s, shorts 3 min) is shown before recording; a refusal shows
  the server's reason in ja/en.
- **Timelines do not reload by themselves**: a realtime hint or the app
  coming back reads the newest page, and a pill says how many wait on top.
- **Gifts** carry one idempotency key per attempt, so a lost answer never
  pays twice.
- What a push shows on the lock screen is the account's preview setting,
  applied by the server (`docs/events/notify.md`).

## Web screens

What is not native opens in a web view, signed in as the account in use. It
plugs in through the seams ([`seams.md`](seams.md)), so pushes, inbox entries
and deep links to web-only paths open there too.

- Sign-in is the identity domain's hand-off: a one-time code
  (`CreateWebHandoff`, 60 seconds, bound to a verifier the first load sends
  in a header) loaded at `/api/auth/handoff?code=…`. The app's tokens never
  reach the web view, and the web session ends with the app's.
- The web view is incognito. Everything web views stored is removed before
  one opens, when the last one closes, before every switch or sign-out
  (`modules/tokuly-web-data`); the app's own API calls send no cookies.
- Links to anywhere else open in the system browser.
- Features move out of `features/web/features.ts` and `links.ts` as they
  become native.

## Builds

- FCM needs `GOOGLE_SERVICES_JSON` (the path of `google-services.json`);
  store builds for iOS set `EXPO_PUBLIC_APNS_ENV=production`.
- The open-source notices (Settings → licences) are written at build time
  by `scripts/licenses.js` to `src/generated/licenses.json` (not checked
  in); native modules add theirs under `modules/*/notices`.
