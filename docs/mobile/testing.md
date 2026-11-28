# Mobile: tests

The commands are in [`apps/mobile/README.md`](../../apps/mobile/README.md), "Checks".

Logic is tested in the shared packages (Vitest). Component tests in the app
(jest-expo, React Native Testing Library) cover the screens that matter and
mock only the edges: the network (`expo/fetch`), device modules, the clock.
A test's query cache comes from `testQueryClient` (`src/testing/query.ts`): it
never starts a garbage-collection timer, so jest exits when the tests are done.

## End to end (Maestro)

The shared flows in `apps/mobile/maestro/flows`, one file per area, run
against a real stack on Android and iOS.

- `.github/workflows/e2e.yml` runs the web Playwright flows and these Maestro
  flows in an Android emulator against the Compose stack.
- `.github/workflows/ios-e2e.yml` runs the same Maestro flows on an iPhone
  simulator against a native stack on a macOS runner. It runs nightly and by
  manual dispatch; it is not a required check, so it does not hold up a PR.
- `.github/workflows/ios.yml` builds the app for an iOS simulator on macOS when
  `main` changes (or by manual dispatch). It checks the generated Xcode
  project, config plugins, pods and Swift modules; it does not run Maestro.

| flow | what it does |
| --- | --- |
| `sign-in` | signs up and in |
| `accounts` | switches accounts |
| `friends-and-map` | accepts a friend (from ともだち under the own profile) and sees them on the map, uses ghost mode and sharing |
| `chat-and-inbox` | opens a chat with an unread message, sends one, receives one from a second account (through the API), starts a chat and opens the inbox |
| `stories` | posts a story from the library and watches it as a friend |
| `shorts` | posts a short, swipes the feed and likes it |
| `video` | opens the video feeds and subscribes to a channel |
| `notes` | reads a friend's note, likes it and replies after agreeing to the rules, posts a note with a poll |
| `live` | finds a friend's scheduled stream, asks for its reminder and chats before it starts |
| `community` | finds a public community, joins it, replies in and likes a thread, starts one and sees the members |
| `school` | sees today's classes and what is due, cheers a friend who is studying, ticks homework off and adds new homework |
| `study-timer` | starts a pomodoro for a subject (puts off the reminder's permission and sees the keep-open note), pauses and resumes it, cheers a friend who is studying, stops and logs it |
| `history` | sees today on the calendar, the day's café, walk and park on the map and in order, writes its memo and deletes the day |
| `places` | finds an own place, checks in there with a few words, finds it by name and deletes it |
| `footprints` | after a walk sees the own level, regions and place in the ranking, and marks the recap seen |
| `settings` | changes the account's settings and avatar on the native screens and finds them kept when they open again |
| `spaces` | finds a friend's room, enters it (audio over LiveKit, Bluetooth headsets declined), raises a hand, sends an emoji, comes back from the mini player and leaves; accepts a hangout invitation, enters, mutes from the mini player on the tabs and leaves |
| `plans` | answers a friend's plan and sees its place on the map, makes a plan on a chosen day with a place and invites the friend |
| `quest` | collects a finished mission's reward with a double tap (one claim), opens the badges and one badge, sees the quest card on the own profile |
| `stamps` | adds a free pack from the store, sees it in マイスタンプ, sends one of its stamps in a chat and puts it on a note |
| `emoji` | opens the emoji picker in a chat and in the note composer, the catalogue and the deck |
| `wallet` | sees the balance, claims the login bonus with a double tap (one claim, the balance goes up once), opens the history and a movement, the purchases |
| `web-screens` | opens a web-only feature from 「もっと見る」 and sees it signed in |

Each flow makes its own accounts and data through the API (`maestro/scripts`), and waits
for the server's state, not for time. Each starts the app afresh through
`subflows/launch.yaml`; its retry covers Android killing a quick relaunch while
cleaning up the previous app process. What a flow taps it first scrolls into view
(`subflows/tap-in-view.yaml`): screens grow, and a tap does not scroll. Android CI uses the
same Pixel 6 as the local one (`tools/agent/android.sh`). Its build logs every tap the app
takes (`src/lib/touch-log.ts`), and the run keeps the device's logcat: a tap that seems lost
shows there whether it reached the app and the tab bar. A change to a
layout also runs its flows on the smallest phone the app is laid out for,
320x640 (`android.sh small <command…>`).

The flows screenshot each main screen into the run's `--test-output-dir`
(`<run>/<flow>/takeScreenshot/<name>.png`): `apps/mobile/maestro/screenshots`
with `pnpm e2e`, the `mobile-screenshots` artifact in CI.

What the flows cannot do is covered by component tests: the emulator has no
camera (the pictures are put in its library; the camera screen has a
component test), and a flow's script cannot upload a video (playback is
covered by the watch screen's component test).
