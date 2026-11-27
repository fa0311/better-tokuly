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
| `@tokuly/core/media`         | uploads: one request or parts side by side, completed or aborted, the wait for processing                                                   |
| `@tokuly/core/chat`          | chat keys, limits, message rules, the message window, realtime events in the cache, unread totals, tokens, times, previews, the outbox      |
| `@tokuly/core/community`     | community keys and options, viewer permissions, page addresses, thread and reply cache edits, pushed community events                       |
| `@tokuly/core/notes`         | notes, list and clip keys, timeline pages and fresh rows, note cache edits, realtime events, text reading, audiences, polls                 |
| `@tokuly/core/notifications` | notify keys, unread counts, inbox cache edits, inbox times, push targets                                                                    |
| `@tokuly/core/plans`         | plan keys and options, days and times of plans in the account's zone, plan rules and the plan form, plan cache edits, pushed plans events   |
| `@tokuly/core/quest`         | quest keys and options, level, mission, badge and streak rules, claim and reroll cache edits, pushed quest events, profile quest cards      |
| `@tokuly/core/school`        | school keys and options, the API's dates and times of day, which classes take place, class and exam countdowns, what is due, the study timer |
| `@tokuly/core/places`        | place keys and options, how each kind of place looks, who it is shown to, "not now" to a suggestion as fingerprints                         |
| `@tokuly/core/history`       | history keys and options, the history's days on the account's calendar, the day's timeline, the encoded path, visits and ways of travelling |
| `@tokuly/core/footprints`    | footprint keys and options, region names, explorer and region levels, readable percentages, new level-ups                                   |
| `@tokuly/core/search`        | search kinds and their names, hit fields, where a hit leads, trend and recent-search keys and options, tag queries, suggestion lists        |
| `@tokuly/core/stories`       | story keys and options, story cache edits                                                                                                   |
| `@tokuly/core/shorts`        | shorts keys, captions, feed contexts, the slide window                                                                                      |
| `@tokuly/core/live`          | live keys and options, gift, chat and poll rules, a stream and its chat in the cache, pushed live events                                    |
| `@tokuly/core/spaces`        | room and hangout keys and options, stage rules, pushed spaces events in the cache, why one is outside, who is heard, tag input              |
| `@tokuly/core/ask`           | ask keys, question-box options, inbox and answer cache edits, pushed ask events, limits, share links, the question as a subject             |
| `@tokuly/core/stamps`        | stamp keys and options of the store and the library, stamp and pack cache edits, pack states and send contexts                              |
| `@tokuly/core/emoji`         | emoji keys and options, the catalogue copy (built, updated, looked up), `:shortcode:`s in text, emoji search, the deck limit                |
| `@tokuly/core/tokens`        | the design tokens as data: both themes' colours, the overlay, radii, the sticker shadow's depth                                             |
| `@tokuly/core/wallet`        | wallet keys and options, balance snapshots (highest revision wins) and pushed wallet events in the cache, coins, money, history days        |
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
