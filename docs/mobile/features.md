# Mobile: how each area works

What the phone app does per area and where the code is. The rules a
contributor follows are in [`apps/mobile/README.md`](../../apps/mobile/README.md);
paths below are under `apps/mobile/src/` unless they say otherwise.

## Sessions

Bearer tokens: the access token in memory, the refresh token
of the active account and those of parked accounts in secure storage
(`features/session/vault.ts`). Who is signed in: `features/session/store.ts`.
Switching accounts, adding one, signing out: `features/session/session.ts`. Each starts the signed-in tree over (a new
API object, realtime connection, query cache and navigation stack), the way
a page load does on the web. A sign-out shows at once; the session's token
is kept aside (`departed`) until the server has removed its push
registrations and ended it, retried on each start when it was offline.
Every use of a parked or departed token waits its turn, so none is ever
sent twice.

## Location

The same rules as the web (`features/map/sharing.ts`): nothing
is sent for an account until it says yes on this device; stopping, a
revoked permission, a sign-out or a switch tell the server, and a stop that
did not reach it is retried. On screen the position is watched and sent at
the server's cadence (`reportDecision` in `@tokuly/core/map`); off screen,
with "always" allowed, the background task sends what the OS delivers
(50 m distance filter, deferred batches, paused while still).

## Chat

The rules are `@tokuly/core/chat`: realtime events go into the
cache through `applyChatEvent` (`features/chat/sync.ts`), timelines page
through `MessagePager`, the unread totals through `UnreadTotals`; the phone
keeps only its live state (typing, read cursors) and its outbox (messages on
their way, photos uploaded first through the media upload flow, sent in
order with an idempotency key, retried by hand). Calls and the AI assistant
stay on the web for now; the conversation says so and opens it.

## Camera and uploads

The camera is VisionCamera (`features/media/camera-screen.tsx`):
a tap takes a photo, holding records (a short: a tap starts and stops), front or
back, flash; the longest recording is the server's limit for the purpose
(stories 60 s, shorts 3 min; every upload's answer brings the server's own),
shown before anything is recorded. A library video that is longer is cut down
first: the trim screen picks the part, and the local module
`modules/tokuly-trim` cuts it on the device (Media3 Transformer on Android,
AVAssetExportSession on iOS; no FFmpeg in the app). A story or a short is a _post_ (`features/media/posts.ts`):
its files are copied into the app's documents, uploaded through the media flow
(`features/media/upload.ts`: one presigned PUT, or the parts the store does not
hold yet), then it is created. PUTs are the OS's own transfers (a background
URL session on iOS); a post that was interrupted (no network, the app closed)
goes on from where it stopped at the next start or return to the front. A post
belongs to the account it was started as and is only ever sent as it: every
request is made with that account's API and only while it is the active one, a
switch stops it first (the feature's `lifecycle.ts`) and it goes on once that account is
back; it is thrown away when that account leaves the device. A refusal shows
the server's reason (MEDIA_TOO_LONG with the limit, in ja/en).

## Stories

The tray (above the chats) and the viewer come from
`@tokuly/core/stories`: tap the sides to go back and on, hold to pause, swipe
sideways between people and down to close; progress bars, a story counts as
watched once it is really shown (`features/stories/seen.ts`), reactions, and
replies, which are direct messages through the chat's outbox. The owner sees
who watched. Stories go to every friend or to the close friends picked for
that story (the story API's audiences).

## Shorts

A tab of its own: the three swipe feeds from `@tokuly/core/shorts`
(its pages, cache edits and stay meter), one short a screen with expo-video
(HLS), the next one loading ahead, sound on or off for the whole feed, likes
(`@tokuly/core/engagement`), comments, the share sheet with the server's link,
hashtag and sound pages, posting (record or pick, trim, caption with hashtag
suggestions, who sees it: public, followers, friends, close friends, a circle,
only me) and the creator's own list with what is still being processed or
failed, and why.

## Videos

Long videos, from 「動画」 in the Me tab's hub, are
`@tokuly/core/video` (keys, cache edits, feeds, the watch page's rules,
playback reporting): the home, subscriptions and trending feeds with
genre chips (holding a card: watch later, save, not interested), the
watch screen with expo-video (HLS, native controls, full screen,
picture-in-picture and the lock screen's controls, from the `expo-video`
config plugin), starting where the viewer left off or at a link's `?t=`,
chapters and description times that jump, likes, watch later, playlists,
subscriptions with their notification level, comments, the playlist
being played and "up next". Playback is reported as the account that
opened the screen and ends with it (`features/video/playback.ts`, its
`lifecycle.ts`); it counts while the picture is on the screen, in the app
or its small window, not as sound behind another app. Channel pages,
playlists, the history and the subscriptions list are native too;
uploading, editing, analytics and watch parties open the studio on the web.

## Notes

A tab of its own: friends (home), following, everyone and
mentions, plus the account's lists and antennas to read (`@tokuly/core/notes`:
the pages, how new notes go on top, the cache edits, realtime events). A
timeline never reads its pages again by itself: a realtime hint or the app
coming back reads the newest page and a pill says how many wait on top. The
card renders the server's rich text (`Note.body`), photos, a video, polls,
quotes, link previews and content warnings; each card follows its note's
topic while on screen, and views are reported once really seen, as the
account that saw them. Likes, renotes, quotes, replies, emoji reactions,
clips, pinning, muting a thread and deleting work here; the composer takes
an audience (with the reply rule of the core), a content warning, a poll
and photos (uploaded as the account that picked them), and keeps drafts on
the server (they follow the account). Channels, editing a note, lists and
antennas, mutes, the numbers and reports stay on the web screen.

## Live

Watching is native (`features/live`, from 「ライブ」 in the Me tab's
hub and from every `/live/<id>` link or push): friends on air, the channels
the viewer subscribes to, everyone public, and what is scheduled. A stream
plays as Low-Latency HLS in expo-video (AVPlayer and ExoPlayer play the
partial segments natively; the app has no WebRTC stack, so WHEP stays the
web's), opened again with fresh URLs when it fails, with the viewer's
heartbeat (`ReportWatching`) while the app is in front. The rules and the
cache are `@tokuly/core/live`: the stream's topic goes through `LiveChats`
(lines, deletions, pins, highlighted gifts, polls, numbers, the end, a
raid), lines of people the viewer blocked stay out, and the chat is read
again after every gap without losing what was pushed meanwhile. Chat with
mentions, slow mode and the server's reason when the viewer may not write;
likes, sharing, reminders, votes, reports; gifts with the tier's colours
and message length, a preview, a confirmation and one idempotency key per
attempt (a gift whose answer was lost is repeated under its key, so
nothing is paid twice). Going live stays on the web screen (`/studio/live`):
the app has no WHIP or RTMP publisher, and the web's studio sets a stream
up and publishes from the camera in the browser. So do clips, the gift
history, buying coins and the moderation tools; the archive opens as a
video, the channel as its page.

## Push

The phones' own services through `expo-notifications`, no Expo push
service: FCM on Android (the build needs `GOOGLE_SERVICES_JSON`, the path of
the Firebase project's `google-services.json`), APNs on iOS
(`EXPO_PUBLIC_APNS_ENV=production` for store builds). Like location sharing,
push is the account's decision on this device (`features/notifications/push.ts`):
the token is registered for an account's session only once it said yes here;
before it signs out or another account takes over, its registration is
removed (the server waits for a registration of that session still being
written). When the server cannot be told, the token itself is thrown away
and the removal is owed: each start tries again as that session, parked on
the device, until it goes through — so the next account never gets the
previous one's pushes. What a push shows
on the lock screen is the account's preview setting, applied by the server;
Android channels (one per category) keep the system's lock-screen rule. A tap
opens the screen the notification is about (`features/notifications/target.ts`),
the inbox otherwise.

## Open-source notices

The notices (Settings → licences, needed for the stores) are
written at build time by `scripts/licenses.js`, which app.config.ts runs:
every production dependency the app bundles with its licence and text, and
the native libraries of the app's own modules (`modules/*/notices`). The
output, `src/generated/licenses.json`, is not checked in.

## Web screens

What the app does not have yet (note channels,
spaces, communities, school, plans, ask, quests, emoji, stamps, the wallet,
most settings, developers, the console) opens in a web view
(`features/web`), signed in as the account in use. It plugs in through
the seams ([`seams.md`](seams.md)): 「もっと見る」 in the Me tab's hub (`hub.ts`), the settings' web
row (`settings-rows.ts`), and the web-only paths (`links.ts`), so pushes,
inbox entries and deep links (`app/+not-found.tsx`) to them open there too. Signing the web view in is the
identity domain's hand-off: the app asks for a one-time code as its own
session (`CreateWebHandoff`, 60 seconds, once, bound to a verifier the
web view's first load sends in a header, so a leaked link signs nobody in)
and loads `/api/auth/handoff?code=…`, which sets the web session's cookie and goes on
to the page; the app's tokens never reach the web view, and the web
session ends with the app's. A code that comes back after another account
took over is dropped. The web view is incognito and keyed by its account's
session; everything web views stored (cookies, on disk too, localStorage,
cache) is removed before one opens, when the last one closes
(`open-screens.ts`: Android's web views share one store, and a screen
that closes late must not take the next one's cookie), before every switch
or sign-out and before a departed session ends (`lifecycle.ts`, with the
local module `modules/tokuly-web-data`), and the app's own
API calls send no cookies (`lib/api.ts`). Links to anywhere else open in the system
browser. Features move out of `features/web/features.ts` and `links.ts`
as they become native.
