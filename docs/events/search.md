# search — events, jobs and configuration

Crate `tokuly-search`, migration `1500_search.sql`, API `tokuly.search.v1`
(`SearchService`, `SearchAdminService`). No realtime events, no
notifications.

`SearchService` works without signing in, except the caller's own history
(`ListRecentSearches`, `DeleteRecentSearch`, `ClearRecentSearches`,
`SetHistoryPaused`). Every RPC of `SearchAdminService` needs the `admin`
role.

## How a document becomes searchable

```
any domain ── outbound::index / unindex(tx, …) ──▶ search.changed ─┐
identity   ── identity.user.*                   ──────────────────┼─▶ consumer "search"
admin      ── admin.content.removed             ──────────────────┘        │
                                                                           ▼
                                             search_documents (the registry, PostgreSQL)
                                                                           │  pending rows, oldest first
                                                                           ▼
                                        indexer (one process at a time, batches per kind)
                                                                           │  add-or-replace / delete tasks,
                                                                           ▼  awaited through the task API
                                              Meilisearch: one index per kind, `<prefix>_<kind>`
```

- **The registry is the truth, the engine is a copy.** Every document is
  stored in `search_documents` with the ID of the event that produced it.
  Event IDs are UUIDv7, so their order is time order: a change older than
  what is stored is dropped. That makes redeliveries harmless and keeps the
  order per document — a delete leaves a tombstone (kept 8 days, one day
  longer than the event stream keeps events), so an older upsert arriving
  late cannot bring the document back.
- **The indexer** copies pending rows into Meilisearch: up to
  `TOKULY_SEARCH_INDEX_BATCH` rows per pass, one "add or replace" and one
  "delete" task per kind, and it waits for the engine to finish them before
  marking the rows as written. A PostgreSQL advisory lock lets one process
  index at a time, which is what keeps the engine in registry order. A
  failed pass marks nothing and is repeated; when Meilisearch is down,
  changes simply accumulate in the registry.
- **One index per kind**, all with the same settings: searchable `title`,
  `handle`, `tags`, `text` (in that order of importance); filterable `kind`,
  `owner_id`, `tags`, `created_at`; sortable `created_at`, `score`, `rank`;
  documents and queries tokenized as Japanese and English
  (`localizedAttributes`, locales `jpn` + `eng`); typos tolerated from 5
  characters (two from 9), never on `handle` and never on numbers; ranking
  rules `words, sort, typo, proximity, attributeRank, exactness, rank:desc,
  wordPosition`. The settings must stay identical across kinds: the "all"
  tab is a federated search, which merges indexes rule by rule.
- **Matching**: a document matches when it contains every word of the query
  (the last word may be a prefix; double quotes make a phrase). The engine's
  default of dropping trailing words is not used: Japanese is segmented into
  morphemes, and 「おさんぽ」 would otherwise match every text containing 「お」.
- **`rank`** orders equally relevant documents:
  `log10(1 + score) + age position on the kind's time scale`. Timely kinds
  (`note`, `video`, `short`, `live`, `clip`, `news`, `answer`, `thread`,
  `space`) lose a factor of ten per 90 days of age; evergreen kinds (`user`,
  `channel`, `playlist`, `stamp_pack`, `place`, `hashtag`, `emoji`,
  `community`) per ten years. Kinds on the same scale
  are comparable when merged; between the two groups a tie goes to the timely
  document. In the "all" tab `user` and `channel` (and slightly `community`,
  `place`, `hashtag`) win near-ties through federation weights.
- **Never returned**, whatever the index holds: documents owned by users the
  caller blocked or was blocked by (`tokuly_social::graph::blocked_ids_either`,
  applied as an engine filter so counts and pages are right, and again on the
  hits), and documents of suspended or deleted owners (kept out of the index
  by the indexer, and filtered on the hits for the moments before it catches
  up).
- **Channel content does not name its owner**: hits of `channel`, `video`,
  `short`, `live` and of a `playlist` whose `extra.channel_id` is set have no
  `owner` (except for the owner), and an `owner_id` search by anyone else
  leaves those kinds (and playlists) out. The same goes for a `clip` with
  `extra.channel_clip = true` (a channel's owner cutting their own channel):
  no `owner` on its hits, and an engine filter keeps it out of someone
  else's `owner_id` search, hits and counts alike. `owner_id` stays in the index for
  blocks, suspensions and deletion.
- **Highlights** are returned as `TextSegment`s of plain text, never markup.
  The engine marks matches with two private-use characters that are removed
  (with control characters) from every document before it is indexed, so
  content can neither inject markup nor forge a highlight.

## Publishing documents (for other domains)

```rust
outbound::index(&mut *tx, SearchDoc { kind: "video".into(), id, title, text, tags, owner_id,
    public: true, image_url, path, created_at, score, extra }).await?;
outbound::unindex(&mut *tx, "video", id).await?;
```

- `kind` is one of `user`, `channel`, `note`, `video`, `short`, `live`,
  `clip`, `playlist`, `stamp_pack`, `place`, `hashtag`, `news`, `answer`
  (ask), `emoji`, `community`, `thread` (community) and `space` (spaces). A change for
  any other kind is logged and dropped. (`user` documents are built by the
  search domain itself, see below; do not publish them.)
- Publish only what everyone may find. `public = false` removes the document.
- `owner_id` is the account the document belongs to: it is what block lists,
  suspensions and account deletion act on. Use `None` (or the nil UUID) for
  content without an owning account.
- `score` is the popularity signal (views, likes, members, installs); publish
  again when it has changed enough to matter. Publishing a document that did
  not change costs one row update and no engine write.
- `extra` is returned to clients as `SearchHit.extra_json`. If it has a string
  `handle`, the handle becomes searchable (exact matches only).
- Texts are cleaned on the way in: single line, control characters removed,
  title at most 200 characters, body 8000, 50 tags of 64.
- Republish everything of your kind when you receive
  `search.reindex.requested` with your kind.

## Events consumed

Durable consumer `search` (`tokuly_search::Search::handle_event`). Every
effect is versioned by the event's ID or states the current truth, so
duplicates and reordering change nothing. An event whose payload cannot be
read is logged and acknowledged.

| Topic | Payload | Effect |
|---|---|---|
| `search.changed` | `tokuly_core::outbound::SearchChange` | the document is stored in, or removed from, the registry |
| `identity.user.created`, `identity.user.updated`, `identity.user.privacy_changed` | `{ user_id }` | the user's own document is rebuilt from identity's `users` row (read-only): display name, handle, bio, avatar URL; searchable only while the account is active and `privacy.discoverable` is true |
| `identity.user.banned` | `{ user_id }` | the user's document is removed and every document they own leaves the indexes (it stays in the registry) |
| `identity.user.unbanned` | `{ user_id }` | their document and everything they own return |
| `identity.user.deleted` | `{ user_id }` | every document they own is removed for good; their recent searches and history setting are deleted; blocked terms and synonym groups they entered lose the reference to them |
| `admin.content.removed` | `{ subject_type, subject_id, … }` | the document of the moderated subject is removed (the owning domain unindexes it as well; this makes the takedown immediate). Subject types without an index are ignored. |

A user document: `kind = "user"`, `id = owner_id = the user's ID`,
`title` = display name (handle when empty), `text` = bio, `path = /u/<handle>`,
`score` = profile views (+1000 when verified),
`extra = { handle, verified }`.

## Events emitted

| Topic | Payload | When |
|---|---|---|
| `search.reindex.requested` | `{ kind, requested_by }` (`tokuly_search::events::ReindexRequested`) | an administrator called `SearchAdminService.Reindex`. Actor: the administrator. The domain owning `kind` republishes all its public documents. |

## Background work (`Search::spawn_workers`, worker processes)

| What | Name | Notes |
|---|---|---|
| Index setup, then the indexer loop | — | `ensure_indexes` (retried until the engine answers), then a pass every `TOKULY_SEARCH_INDEX_INTERVAL_MS`, at once when this process stored a change, back to back while a pass is full |
| Event consumer | durable `search` | topics above |
| User backfill | job `search.reindex.users` (`tokuly_search::users::ReindexUsersJob {}`) | rebuilds every user document from identity's data, 500 accounts per step; queued by `Reindex` of the user kind. Queue it once after the first deployment to make existing accounts searchable. |
| Synonyms | job `search.synonyms.apply` (`tokuly_search::maintenance::ApplySynonymsJob {}`) | applies the stored synonym groups to every index; queued by `SaveSynonymGroup` / `DeleteSynonymGroup` |
| Trending | timer, every `TOKULY_SEARCH_TRENDING_REFRESH_SECONDS` | `Search::refresh_trending` |
| Retention | timer, hourly | removes tombstones older than 8 days that the engine has seen, and popular queries that have not qualified for 30 days |

## Trending, completions and history

- A search counts towards trending when it is the first page of a signed-in
  user's search, found something, was not flagged `skip_history`, the user has
  not paused their history, and the query is at most 40 characters and not on
  the blocklist. Guests never count.
- Counters live in Valkey per hour and query: a HyperLogLog of the users who
  searched it and a sorted set of queries by distinct users
  (`search:{<prefix>}:trend:u:<hour>:<query>`, `…:trend:z:<hour>`), expiring
  after the window. Nothing else records who searched what.
- The refresh folds the window (`TOKULY_SEARCH_TRENDING_WINDOW_HOURS`, recent
  hours weigh more) into `search_popular_queries`. A query qualifies only when
  at least `TOKULY_SEARCH_TRENDING_MIN_USERS` *distinct* users searched it
  within the window, so one person cannot make something trend. The top
  `TOKULY_SEARCH_TRENDING_SIZE` are the trending list; all qualifying queries
  are the popular completions of the typeahead — a query only one person
  typed is never shown to anyone else.
- Blocked terms (`SearchAdminService.AddBlockedTerm`) keep every query
  containing them out of trending and completions, immediately.
- Recent searches: one entry per distinct query (case-insensitive), the
  newest 50 per user, in `search_recent`; included in the personal data
  export (section `search`).
- Analytics: one `search.query` event per first page (`value` = number of
  matches, `props = { kind, tags_only }`). The query text is not recorded.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_SEARCH_INDEX_PREFIX` | `tokuly` | Prefix of the index names (`<prefix>_<kind>`) and of the search keys in Valkey. |
| `TOKULY_SEARCH_INDEX_BATCH` | `1000` | Registry rows written to the engine per indexer pass. |
| `TOKULY_SEARCH_INDEX_INTERVAL_MS` | `500` | How often an idle indexer looks for work. |
| `TOKULY_SEARCH_RATE_PER_MINUTE` | `120` | Searches per minute for one signed-in user. |
| `TOKULY_SEARCH_ANONYMOUS_RATE_PER_MINUTE` | `40` | Searches per minute for one client address without signing in. |
| `TOKULY_SEARCH_SUGGEST_RATE_PER_MINUTE` | `300` | Typeahead (and trending) requests per minute per user; guests get half per client address. |
| `TOKULY_SEARCH_TRENDING_MIN_USERS` | `5` | Distinct signed-in users needed before a query trends or is offered as a completion. |
| `TOKULY_SEARCH_TRENDING_WINDOW_HOURS` | `24` | Hours of searches the trending list looks back on (1–168). |
| `TOKULY_SEARCH_TRENDING_HALF_LIFE_HOURS` | `6` | Age at which a search counts half. |
| `TOKULY_SEARCH_TRENDING_REFRESH_SECONDS` | `60` | Seconds between two computations of the trending list. |
| `TOKULY_SEARCH_TRENDING_SIZE` | `20` | Queries in the trending list (1–100). |

Meilisearch itself is configured by the core settings `TOKULY_MEILI_URL` and
`TOKULY_MEILI_KEY`. Developed against Meilisearch 1.57 (needs the
`attributeRank` / `wordPosition` ranking rules, `localizedAttributes` and
federated search with merged facets).

## For the server crate

```rust
let search = Search::new(core.clone(), identity.clone(), cfg.search)?;  // also registers the "search" data exporter
let router = search.register(router);
search.setup().await?;        // migrate step: creates the indexes and applies their settings (idempotent)
search.spawn_workers();       // worker processes
```

API processes need no setup of their own: a search that finds the indexes
missing creates them and tries again.
