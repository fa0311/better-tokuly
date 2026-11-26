# Shared packages: what is where

The rules for what may live in `packages/` are in
[`packages/README.md`](../packages/README.md).

`@tokuly/core` has one entry point per area (`packages/core/src/<area>/index.ts`,
listed in its `package.json`), each with its own tests:

| entry                        | what it holds                                                                                                                               |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `@tokuly/core/format`        | ages, durations, distances, speeds, dates and times on a zone's clock                                                                       |
| `@tokuly/core/errors`        | an API reason or a refused upload → the text to show                                                                                        |
| `@tokuly/core/engagement`    | subjects, counts and the viewer's reactions, a tap shown before the answer, realtime counters, engagement keys                              |
| `@tokuly/core/query`         | the retry rule, `ClientSource`, `nextCursor`, pushes that arrive while a snapshot is read                                                   |
| `@tokuly/core/session`       | device accounts                                                                                                                             |
| `@tokuly/core/profile`       | profile keys and options, the viewer's relationship                                                                                         |
| `@tokuly/core/friends`       | follow and circle keys, the follow summary, what to re-read after a change                                                                  |
| `@tokuly/core/map`           | map keys, pin and ghost rules, clustering, live edits, fix reporting, style                                                                 |
| `@tokuly/core/chat`          | chat keys, limits, message rules, the message window and its paging, realtime events in the cache, unread totals, tokens, times, previews   |
| `@tokuly/core/community`     | community keys and options, viewer permissions, page addresses, thread and reply cache edits, pushed community events                       |
| `@tokuly/core/notes`         | notes, list and clip keys, timeline pages and fresh rows, note cache edits, realtime events, text reading, audiences, polls                 |
| `@tokuly/core/notifications` | notify keys, unread counts, inbox cache edits, inbox times, push targets                                                                    |
| `@tokuly/core/plans`         | plan keys and options, days and times of plans in the account's zone, plan rules and the plan form, plan cache edits, pushed plans events   |
| `@tokuly/core/search`        | search kinds and their names, hit fields, where a hit leads, trend and recent-search keys and options, tag queries, suggestion lists        |
| `@tokuly/core/stories`       | story keys and options, story cache edits                                                                                                   |
| `@tokuly/core/shorts`        | shorts keys, captions, feed contexts, the slide window                                                                                      |
| `@tokuly/core/live`          | live keys and options, gift, chat and poll rules, a stream and its chat in the cache, pushed live events                                    |
| `@tokuly/core/spaces`        | room and hangout keys and options, stage rules (who talks, who moderates, its order), pushed spaces events in the cache, tag input          |
| `@tokuly/core/video`         | video keys, cache edits, feeds, the watch page's rules (start position, chapters, description links), playlists, playback reporting, clocks |

## Using it from an app

Shared logic takes the app's client and i18next instance as arguments:

```ts
// an app's hook: the shared options, the app's client, the app's extras
export function useStory(storyId: string) {
  return useQuery({ ...storyQueries.story(storyClient, storyId), enabled: Boolean(storyId) });
}

// an app's error text: the shared rules, the app's i18next instance
export const errorMessage = (error: unknown) => errors.errorMessage(i18n, error);
```
