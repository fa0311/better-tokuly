# Mobile: tests

The commands are in [`apps/mobile/README.md`](../../apps/mobile/README.md), "Checks".

Logic is tested in the shared packages (Vitest). Component tests in the app
(jest-expo, React Native Testing Library) cover the screens that matter and
mock only the edges: the network (`expo/fetch`), device modules, the clock.

## End to end (Maestro)

The flows in `apps/mobile/maestro/flows`, one file per area, run against a
real stack. The End to end workflow (`.github/workflows/e2e.yml`) builds a
release APK for the emulator and runs them against the Compose stack.

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
| `settings` | changes the account's settings and avatar on the native screens and finds them kept when they open again |
| `spaces` | finds a friend's room, enters it (audio over LiveKit, Bluetooth headsets declined), raises a hand, sends an emoji, comes back from the mini player and leaves; accepts a hangout invitation, enters, mutes from the mini player on the tabs and leaves |
| `plans` | answers a friend's plan and sees its place on the map |
| `web-screens` | opens a web-only feature from 「もっと見る」 and sees it signed in |

The flows screenshot each main screen into the run's `--test-output-dir`
(`<run>/<flow>/takeScreenshot/<name>.png`): `apps/mobile/maestro/screenshots`
with `pnpm e2e`, the `mobile-screenshots` artifact in CI.

What the flows cannot do is covered by component tests: the emulator has no
camera (the pictures are put in its library; the camera screen has a
component test), and a flow's script cannot upload a video (playback is
covered by the watch screen's component test).
