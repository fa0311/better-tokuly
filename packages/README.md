# Shared packages

Code that every Tokuly app (web, mobile, desktop) runs the same way.

| package            | what it holds                                                                         |
| ------------------ | ------------------------------------------------------------------------------------- |
| `@tokuly/api`      | the generated API (Protobuf + Connect RPC), the session and its token refresh, errors |
| `@tokuly/realtime` | the realtime connection (WebTransport, WebSocket fallback) and its frames             |
| `@tokuly/i18n`     | the strings of the areas every app has (ja first, en), the language rules             |
| `@tokuly/core`     | app logic without UI or platform: one entry point per area (below)                    |

`@tokuly/core` entry points, each with its own `index.ts` and tests:

| entry                        | what it holds                                                                                                                               |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `@tokuly/core/format`        | ages, durations, distances, speeds                                                                                                          |
| `@tokuly/core/errors`        | an API reason or a refused upload → the text to show                                                                                        |
| `@tokuly/core/engagement`    | subjects, counts and the viewer's reactions, a tap shown before the answer, realtime counters, engagement keys                              |
| `@tokuly/core/query`         | the retry rule, `ClientSource`, `nextCursor`                                                                                                |
| `@tokuly/core/session`       | device accounts                                                                                                                             |
| `@tokuly/core/profile`       | profile keys and options, the viewer's relationship                                                                                         |
| `@tokuly/core/friends`       | follow and circle keys, the follow summary, what to re-read after a change                                                                  |
| `@tokuly/core/map`           | map keys, pin and ghost rules, clustering, live edits, fix reporting, style                                                                 |
| `@tokuly/core/chat`          | chat keys, limits, message rules, the message window and its paging, realtime events in the cache, unread totals, tokens, times, previews   |
| `@tokuly/core/notifications` | notify keys, unread counts, inbox cache edits, inbox times, push targets                                                                    |
| `@tokuly/core/stories`       | story keys and options, story cache edits                                                                                                   |
| `@tokuly/core/shorts`        | shorts keys, captions, feed contexts, the slide window                                                                                      |
| `@tokuly/core/video`         | video keys, cache edits, feeds, the watch page's rules (start position, chapters, description links), playlists, playback reporting, clocks |

## What belongs here

> 「共通化の為に責務が多いコンポーネントになったりするのは本質的じゃない。工数を少なく出来るのは
> 嬉しいが、バグが増えたり保守性も考慮するべきだ」 — the product owner

Logic only:

- query keys, query options (`{ queryKey, queryFn, ... }`, no hooks) and mutation functions;
- pure domain rules: who may do what, which list something is in, unread counting, grouping,
  validation, limits;
- formatting (dates, distances, counts, relative times) and error reason → message;
- strings (`@tokuly/i18n`);
- cache edits for what the user does and what realtime events report (functions of a
  `QueryClient` from `@tanstack/query-core`, or pure functions of the cached data);
- planning that does not touch a platform (which slides to load, how to page).

Never:

- UI of any kind: no React components, no hooks, no "universal" components;
- the platform: no `window`, `document`, `navigator`, storage, cookies, `Platform.OS`, file
  pickers, and no branches on which platform this is. When logic needs something only the
  platform has, the function takes the value as a plain argument (the clock is fine);
- set-up state: no global singletons configured at start-up, no adapter registries. A function
  that speaks takes the app's i18next instance (`errorMessage(i18n, error)`,
  `chatTimes(i18n)`); a query takes the app's client as a `ClientSource` (`() => client`);
- anything from `apps/*`, a router or React's DOM.

Each module has one job and a small public surface. When one grows a second job, split it.
Small duplication in app code is better than a shared abstraction with an option per platform.
Move something here when a second app needs it, not before.

`packages/core/src/boundaries.test.ts` fails when a shared package imports UI, a router, an app
or a platform, touches browser globals, or its `tsconfig.json` includes the DOM library.

```ts
// an app's hook: the shared options, the app's client, the app's extras
export function useStory(storyId: string) {
  return useQuery({ ...storyQueries.story(storyClient, storyId), enabled: Boolean(storyId) });
}

// an app's error text: the shared rules, the app's i18next instance
export const errorMessage = (error: unknown) => errors.errorMessage(i18n, error);
```
