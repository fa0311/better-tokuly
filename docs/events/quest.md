# quest — events, jobs and configuration

Crate `tokuly-quest`, migration `1900_quest.sql`, API `tokuly.quest.v1`
(`QuestService`, `MissionService`, `BadgeService`, `StreakService`,
`LeaderboardService`, `QuestAdminService`, realtime `QuestEvent`). Depends on
`identity`, `social` and `wallet`; everything else reaches it as events.

The domain rewards using the app with friends: XP and levels, daily and
weekly missions, badges, friend streaks, leaderboards and monthly seasons.
Nothing is "done" through its RPCs — they read progress and collect rewards.
XP is never taken away, a day off costs nothing but the streaks that needed
it, and the only thing that can be bought is one streak restore a month.

## How an event becomes a reward

```
EVENTS stream (domain events)      ANALYTICS stream (analytics events)
        │ consumer "quest"                  │ consumer "quest"
        └──────────────┬────────────────────┘
                       ▼
        the rule table (rules.rs): what is this event worth?
        a reader (payloads.rs) says who did it, how much, towards whom
                       │
                       ▼
   Valkey: one script per deed — applied once per event ID, XP capped
   per source and local day, counters added            (no PostgreSQL)
                       │  TOKULY_QUEST_FLUSH_DELAY_MS later, per player
                       ▼
   PostgreSQL, one transaction: XP, level, counters, mission progress,
   badges whose threshold was crossed
                       │
                       ▼
   level-up coins (wallet), realtime QuestEvent, analytics
```

Something done *towards a friend* additionally records the doer's part of
the streak day with that friend (PostgreSQL, once per friend and local day).

Properties worth knowing:

- **Idempotent.** A deed is applied once per event ID (`seen` markers, 48 h);
  events older than 47 h are not counted at all, so a replayed stream cannot
  pay twice. The streak machine ignores parts it already has, at any age.
- **Cheap.** An event costs one Valkey script call. PostgreSQL is written
  once per player and flush, not once per event.
- **Tolerant.** A topic the table does not know and a payload that cannot be
  read are no-ops (the latter logged at `warn`). Payload structs are the
  crate's own, with every field optional.
- **On the player's calendar.** Days are local dates in `users.time_zone`
  (default `Asia/Tokyo`), taken from the event's `occurred_at`. An event that
  arrives late counts for the day it happened.
- Deleted and banned accounts earn nothing.
- **What is hidden earns nothing.** XP, counters, badges and streaks are
  seen by other people (leaderboards, badge lists, the streak a friend
  watches), so an event whose producer hides who did it, or from whom, is
  not counted at all: private notes, close-friends stories, answers only the
  asker may read, comments written as a channel, activity entries recorded
  as private (a check-in in ghost mode, a new region while footprints are
  hidden), and — for players who show their footprints to nobody — every
  rule that follows from where they have been.

## Events consumed

Two durable consumers named `quest`, one per stream.

### The rule table

`tokuly_quest::rules::RULES` — the same table `QuestService.ListEarningRules`
serves to clients. XP counts against the daily cap of the rule's source;
"counts" are the metrics missions, counted badges and leaderboards read;
"streak" means it is something done towards a friend.

Domain events (`EVENTS`):

| Rule | Topic | Who earns | XP | Source | Counts | Streak |
|---|---|---|---|---|---|---|
| `daily.joined` | `identity.user.created` `{ user_id }` | the new user | 10 | daily | `joined` | |
| `daily.bonus` | `wallet.balance.changed` with `reason = "daily_bonus"` | `user_id` | 10 | daily | `daily_bonuses` | |
| `friends.added` | `social.friend.accepted` `{ user_id, friend_id }` | both | 20 | friends | `friends_added` | |
| `friends.bump` | `social.bump.created` `{ user_ids, friends }`, only with `friends: true` | both | 15 | friends | `bumps` | each towards the other |
| `friends.intro` | `social.intro.created` `{ inviter_id }` | the inviter | 30 | friends | `intros` | |
| `friends.map_reaction` | `notify.requested`, kind `social.map_reaction` | `actor_id` | 3 | friends | `map_reactions` | towards `user_id` |
| `friends.buzz` | `notify.requested`, kind `social.buzz` | `actor_id` | 1 | friends | | towards `user_id` |
| `friends.knock` | `notify.requested`, kind `social.knock` | `actor_id` | 1 | friends | | towards `user_id` |
| `chat.call` | `chat.call.ended` `{ duration_seconds, participant_ids }`, 60 s or longer, two or more participants | every participant | 10 | chat | `calls`, `call_minutes` (+minutes) | two-person calls: towards the other |
| `stamps.installed` | `stamp.pack.installed` `{ user_id }` | `user_id` | 5 | stamps | `packs_installed` | |
| `notes.posted` | `notes.note.created` `{ author_id, visibility, reply_to_id }`, not a reply, not private | `author_id` | 8 | notes | `notes_posted` | |
| `notes.replied` | `activity.recorded`, kind `note.replied`, not private | `user_id` | 3 | notes | | towards `with_user_ids` |
| `notes.story` | `notes.story.created` `{ owner_id, audience }`, not for close friends only (`audience = 2`) | `owner_id` | 8 | notes | `stories_posted` | |
| `creators.video` | `video.video.published` `{ owner_id, first_publication }`, only with `first_publication: true` | `owner_id` | 40 | creators | `videos_published` | |
| `creators.stamp_pack` | `stamp.pack.published` `{ creator_id, official }`, not official | `creator_id` | 40 | creators | `packs_published` | |
| `reactions.reaction` | `engagement.reaction.changed`, a like or emoji where there was none (`previous_kind = 0`, `kind` 1 or 3) | `user_id` | 1 | reactions | `likes_given` | towards `subject_owner_id` |
| `reactions.reacted` | the same event | `subject_owner_id` | — | | `likes_received` | |
| `reactions.comment` | `engagement.comment.published` `{ author_id, subject_owner_id, channel_id }`, not written as a channel | `author_id` | 3 | reactions | `comments` | towards `subject_owner_id` |
| `reactions.share` | `engagement.share.recorded` `{ user_id }` | `user_id` | 2 | reactions | `shares` | |
| `together.answer` | `ask.answer.published` `{ owner_id, audience, first }`, only with `first: true` and not private (`audience = 3`) | `owner_id` | 5 | together | `questions_answered` | |
| `together.vote` | `ask.round.completed` `{ user_id, votes }`, at least one vote | `user_id` | 5 | together | `compliment_votes` (+votes, at most 50) | |
| `map.checkin` | `activity.recorded`, kind `geo.checkin`, not private | `user_id` | 10 | map | `check_ins` | towards the tagged `with_user_ids` (up to 8) |
| `map.new_region` | `activity.recorded`, kind `geo.new_region`, not private | `user_id` | 30 | map | `regions_explored` | |

The two `ask.*` rows follow what the ask domain documents in
`docs/events/ask.md`.

Analytics events (`ANALYTICS`, subject `an.<name>`; the consumer filters on
exactly these names, so nothing else of the analytics traffic reaches it):

| Rule | Name | Who earns | XP | Source | Counts | Streak |
|---|---|---|---|---|---|---|
| `chat.message` | `chat.message.send` (user = sender, subject = conversation, props `{ group }`) | the sender | 2 | chat | `messages` | direct conversations: towards the other member |
| `stamps.sent` | `stamp.send` (user = sender) | the sender | 2 | stamps | `stamps_sent` | |
| `reactions.note_like` | `note.like` (user = who liked, owner = the note's author) | who liked | 1 | reactions | `likes_given` | towards the author |
| `reactions.note_liked` | the same event | the author | — | | `likes_received` | |

**Expected topics.** These rules are ready for domains that are still being
built. They credit the event's *actor* (what the producer passes to
`events::emit`), stay silent until the topic exists, and cost nothing until
then. If a domain names its event differently, the fix is one row in
`rules.rs`:

| Rule | Expected topic | Payload read | XP | Source | Counts |
|---|---|---|---|---|---|
| `creators.short` | `shorts.short.published` | actor | 20 | creators | `shorts_posted` |
| `creators.live` | `live.stream.started` | actor | 30 | creators | `lives_started` |
| `map.explored` | `geo.footprint.expanded` | `{ user_id, new_cells }`; not for players who hide their footprints | 1 per cell | map | `explored_cells` (+cells) |
| `map.new_place` | `geo.visit.started` | `{ user_id, first_visit }` (counts unless `first_visit: false`); not for players who hide their footprints | 5 | map | `places_visited` |
| `study.session` | `school.session.completed` | `{ user_id, minutes }` or `{ duration_seconds }`, a minute or more, at most 12 h | 1 per minute | study | `study_minutes` (+minutes), `study_sessions` |
| `together.plan` | `plans.plan.created` | actor | 10 | together | `plans_created` |
| `together.space_join` | `spaces.room.joined` | actor | 5 | together | `spaces_joined` |
| `together.space_open` | `spaces.room.opened` | actor | 10 | together | `spaces_hosted` |
| `together.thread` | `community.thread.created` | actor | 8 | together | `threads_posted` |

Two things earn nothing on purpose. Asking a question: questions are
anonymous, the ask domain does not say who asked, and anything a rule gave
the asker — let alone a streak moving — would be a hint. And spending coins.

What the domain credits by itself: `streaks.day` — 5 XP (source `streaks`)
and `streak_days` +1 for each of two friends when a streak day completes;
`friends_reached` +1 the first time a day a player does something towards a
given friend.

A rule is **live** while it fired for anyone within
`TOKULY_QUEST_LIVE_RULE_DAYS` (`quest_rule_activity`). Missions and metric
leaderboards are only offered for live rules, so a feature that does not
exist on a server, or that nobody uses, never shows up there.

### XP sources and daily caps

| Source | Cap per local day | | Source | Cap per local day |
|---|---|---|---|---|
| `friends` | 100 | | `map` | 60 |
| `chat` | 40 | | `study` | 60 |
| `stamps` | 20 | | `together` | 60 |
| `notes` | 40 | | `streaks` | 25 |
| `creators` | 80 | | `daily` | 20 |
| `reactions` | 30 | | `missions` | none (limited by the missions themselves) |

Counters are not capped: the twentieth stamp of a day earns no XP but still
counts for "send 5 stamps" and for the stamp badges.

### Housekeeping topics

| Topic | Reaction |
|---|---|
| `identity.user.deleted` `{ user_id }` | Everything the domain holds about the player is deleted, including the streaks they had with others. Counters still waiting in Valkey are dropped when their turn comes. |
| `social.friend.removed` `{ user_id, friend_id }` (unfriend or block) | The streak of the two is deleted for good; both are pushed `STREAK_CHANGE_ENDED`. Only a streak that existed when the event occurred: delivered again, or after the two made up and started over, the event leaves the new streak alone. |
| `chat.conversation.created` `{ conversation_id, kind, member_ids }` | For direct conversations (`kind = 1`): remembers the two members (`quest_direct_chats`), because `chat.message.send` names the conversation and not the recipient. |

## Levels

Level 2 takes 100 XP and every further level 50 XP more than the one before:
level `L` is reached at `25 × (L − 1) × (L + 2)` total XP (level 10 at 2,700,
level 20 at 10,450, level 50 at 63,700). There is no top level.

Each level-up grants `TOKULY_QUEST_LEVEL_UP_COINS`, every tenth level
`TOKULY_QUEST_MILESTONE_LEVEL_COINS` instead.

## Friend streaks: what a day is

A streak day is a **calendar date**. A friend does their part for a date by
doing something towards the other *while it is that date on their own clock*
(`users.time_zone`). The date is complete once both did their part, and the
streak is the number of consecutive complete dates.

- Two friends in different time zones are on different dates for part of
  every day, so the two parts of one date may arrive up to a day apart, and
  one of them may already be working on the next date. A time zone never
  shortens anybody's day: each has their whole local day, until their own
  midnight.
- Dates are settled in order. The date after the last settled one is
  complete (both parts are there), missed (somebody who did not do their part
  has run out of day) or still open.
- A running streak therefore ends at the **earliest local midnight among the
  friends whose part of the open date is missing** (`Streak.expires_at`).
  Within four hours of it the streak shows the hourglass
  (`STREAK_STATE_EXPIRING`).
- Events arrive a moment after they happen, so a running streak is only
  declared lost `TOKULY_QUEST_STREAK_GRACE_MINUTES` after that midnight (and
  the sweep waits that long after a start, so a backlog is handled first).
  Read through the API, a streak past its deadline is already lost.
- A streak that ended can be restored for 24 hours, by either friend, for
  `TOKULY_QUEST_STREAK_RESTORE_COINS`, once per player and calendar month
  (the player's own). Restoring forgives every date up to yesterday and gives
  both until tonight. Streaks of a single day are not offered.
- A restore is an **order** (`quest_streak_restores`): it is stored, with the
  loss it is for, before any coins move; then the wallet is charged
  (idempotent on the order) and the streak restored. An order that was
  interrupted is finished from wherever it is seen next — `RestoreStreak`,
  `GetStreak`, `ListStreaks`, the `payouts` worker — however much later: it
  is applied, or, when there is nothing left to restore (the friend was
  faster, the friendship ended), refunded and withdrawn, which also gives
  the month's restore back. An order that cannot be paid is withdrawn.
- Milestones: 3, 7, 30, 100, 365, 500 and 1,000 days, each with a badge for
  both.

The state machine is `streaks/days.rs`; it is pure and tested against the
definition for random histories in random pairs of zones.

## Events emitted

No domain events of its own. Through `tokuly_core::outbound`:

### Notification kinds

| Kind | Recipient | Channels | Notes |
|---|---|---|---|
| `quest.badge` | the player | inbox + push | A badge was earned. `data = { badge_key, family, tier }`, path `/quest/badges/<key>`. |
| `quest.streak_expiring` | a friend whose own part is missing less than 4 h before their own midnight | inbox + push, **not urgent** (quiet hours hold it back) | Once per player and streak day; only for streaks of at least `TOKULY_QUEST_STREAK_REMINDER_MIN_DAYS`; not when the player switched `streak_reminders` off. Actor: the friend (so a muted friend stays silent). Collapse key `quest.streak:<lo>:<hi>`, `data = { friend_id, days, expires_at }`. |
| `quest.streak_restored` | the other friend | inbox + push | Actor: who restored. `data = { friend_id, days }`. |
| `quest.season` | every participant of a finished season who had rivals | winners: inbox + push; others: inbox only | Collapse key `quest.season:<YYYY-MM>`, `data = { season, rank, group_size, badge_won }`. |

Losing a streak sends no notification on purpose: the only thing to do about
it costs coins.

The notify domain files kinds by prefix; `quest.` is not in its list and
falls into `system` (see "Requests").

### Activity entries

| Kind | Audience | When |
|---|---|---|
| `quest.badge` | friends — private while the player keeps their progress off their profile | A badge that comes with a title was earned (secret badges never). `data = { badge_key, emoji, name_ja, name_en, tier }`. |
| `quest.level_up` | same | Every fifth level. `data = { level }`. |

## Realtime pushes

`ServerFrame.quest` (`QuestEvent`), always to the player concerned:

| Event | When |
|---|---|
| `xp_gained` | Earnings were written: everything since the last push, per source, with the new level standing. At most one per player per flush delay. |
| `level_up` | With the coins granted. |
| `badge_unlocked` | A badge was earned. |
| `mission_completed` | A mission reached its target (the reward waits to be claimed). |
| `streak_changed` | To both friends, each with their own view: `EXTENDED`, `MILESTONE`, `EXPIRING` (when a reminder went out), `LOST`, `RESTORED`, `ENDED` (the friendship ended). |

## Profile cards

Through `tokuly_identity::ProfileExtension`:

| Kind | Shown to | Content |
|---|---|---|
| `quest.level` | everyone who may see the profile, unless the owner switched `show_on_profile` off | Title `レベルN`, text = the selected title (称号), emoji = the first showcased badge's (else ⭐), path `/u/<handle>/quest`, `data = { level, xp, title_ja, title_en, title_badge, badges }` where `badges` is a JSON array `[{ key, emoji, tier, name_ja, name_en }]` of the showcase (up to 5). |
| `quest.streak` | the friend the streak is with, while it runs | Title `N日連続`, emoji 🔥 or ⌛, `data = { days, best_days, state, waiting_on, next_milestone, expires_at }`. |

## Missions

- Three daily missions (local midnight) and one weekly (Monday), drawn from
  `missions/pool.rs`. A mission counts one metric over its period.
- A mission is only drawn for a player who can do it: they have the friends
  it takes (nobody without friends is asked to bump one), creator missions
  go to people who have created before, and the metric must be fed by a live
  rule. A day mixes one easy, one normal and one harder mission, never two
  counting the same thing. Draws are a function of player, period and
  attempt.
- Sets are drawn when the player opens them and with the first earnings of a
  day; either way the whole period counts.
- Rewards: daily easy 5 coins + 10 XP, normal 10 + 20, hard 20 + 40; weekly
  easy 20 + 40, normal 30 + 60, hard 50 + 100; three weekly missions also
  grant a badge. Claiming every daily mission of a day adds 15 coins + 30 XP.
- One reroll per local day; the replacement starts from zero.
- A finished mission stays claimable for 7 days after its period.

## Badges

`badges/catalog.rs`: 149 badges in 13 families (`friends`, `explore`, `chat`,
`stamps`, `creators`, `study`, `streaks`, `seasons`, `journey`, `missions`,
`together`, `secret`, `special`), most of them lines of bronze / silver /
gold counted at three thresholds. Gold tiers and some single badges unlock a
title.

- Counted badges unlock the moment their threshold is crossed (and are
  caught up when the catalogue is opened, for badges added later).
- **Secret** badges are listed without name, description or real key until
  earned (an opaque `secret_…` key stands in; asking for the real key of one
  the caller does not own finds nothing).
- **Who sees what.** The showcase and the title are what a player chose to
  show: everyone sees them. The full list, with when each badge was earned,
  is for the player and their friends (a badge and its date say when
  somebody first published a video or left town); `ListUserBadges` returns
  only the showcase to anybody else.
- **Rarity** is the share of active players (XP within 30 days) owning a
  badge, recomputed by a worker. It is published only while at least 10
  players are active: below that a share says who owns a badge, not how
  rare it is.
- Staff award and revoke by hand (`QuestAdminService`), never for
  themselves (`SELF_AWARD`).

## Leaderboards and seasons

- A board ranks the caller and their friends only. Players who switched
  `show_on_leaderboards` off are left out (and do not count in others'
  season groups); they still see their own row and get their own result.
- Boards: `xp` (week, season, all time), `streak` (longest running streak),
  and, while their events exist, `explored_area` (map cells) and
  `study_time` (minutes).
- A season is a calendar month, counted on each player's own calendar. It is
  finalized 12 hours after the month ended in UTC — when it is over in every
  time zone — with a snapshot of every participant's XP and rank among their
  friends who took part. The top third of each such group, rounded up, wins
  the badge of that month (`season_01` … `season_12`; winning the same month
  again counts on the same badge); groups of one have no contest. First
  place in a group of three or more is a `season_champion`.

## Wallet

| Reason | Idempotency key | Movement |
|---|---|---|
| `quest.level_up` | `quest.level:<user>:<level>` | grant |
| `quest.mission` | `quest.mission:<mission id>` | grant |
| `quest.daily_bonus` | `quest.daily_bonus:<user>:<date>` | grant |
| `quest.streak_restore` | `quest.streak_restore:<order id>` | spend (refunded with reason `quest.streak_restore.refund` when there turned out to be nothing to restore) |

Coins settle exactly once and are never lost:

- Grants follow the change they reward, which records that they are owed
  (`paid_at` on a claimed mission, `bonus_paid_at` with the amount fixed at
  the time, `rewarded_level` on the profile). What is still owed is granted
  from every later entry point: the claim repeated, `ListMissions`,
  `GetProgress`, the player's next earnings, and the `payouts` worker.
- A charge is preceded by its order (above), so a charge without a restore
  cannot survive: it is finished or refunded.

## Analytics events

| Name | Value / props |
|---|---|
| `quest.xp` | value = XP written, `{ source }` — one per source and flush |
| `quest.level_up` | value = level, `{ from, coins }` |
| `quest.badge` | `{ badge, family, tier }` |
| `quest.mission.complete` | subject `("mission", id)`, `{ mission, period }` |
| `quest.mission.claim` | subject `("mission", id)`, value = coins, `{ mission, period, xp, badge }` |
| `quest.mission.daily_bonus` | value = coins, `{ xp }` |
| `quest.mission.reroll` | subject `("mission", id)`, `{ from, to }` |
| `quest.streak.milestone` | value = days, `{ users }` |
| `quest.streak.restore` | value = coins, `{ friend_id, days }` |
| `quest.season.badge` | value = rank, `{ season, group_size, champion }` |

None of these names is in the rule table, so the domain never feeds itself.

## Public Rust API

```rust
// Give a player a badge of the catalogue. Returns whether they got it now.
// Notifies and pushes. Errors: BADGE_NOT_FOUND, USER_NOT_FOUND.
tokuly_quest::badges::award(core: &Core, user_id: Uuid, badge_key: &str) -> AppResult<bool>

// Optional bridge: who a direct conversation is between (see "Requests").
trait tokuly_quest::DirectChats { async fn members(&self, conversation_id: Uuid) -> AppResult<Option<(Uuid, Uuid)>>; }
Quest::set_direct_chats(&self, lookup: Arc<dyn DirectChats>)
```

Worker entry points, also what tests drive: `Quest::handle_event`,
`handle_analytics`, `flush_user`, `flush_due`, `drain_analytics`,
`sweep_streaks`, `finalize_season`, `finalize_due_seasons`,
`recompute_rarity`, `pay_stragglers`, `reward_stragglers`, `prune`.

`Quest::new` registers the `ProfileExtension` above and a `DataExporter`
named `quest` (profile, XP by day, counters, badges, missions, streaks,
restores, season results).

## Background work

`Quest::spawn_workers` starts (worker processes only; every job may run in
several processes):

| Worker | Schedule | What it does |
|---|---|---|
| event consumer `quest` | on events | `handle_event`, 32 at a time |
| analytics consumer `quest` | on analytics events | `handle_analytics`, batches of 200 |
| flush | every 500 ms | Days whose delay is over → PostgreSQL; one process per player at a time |
| streak sweep | every minute, first after the grace | Ends overdue streaks, sends "expires soon" reminders |
| payouts | every minute | Coins of mission claims and level-ups a stopped process left ungranted; streak restores it left unfinished |
| seasons | every 10 minutes | Finalizes months that are over everywhere (looks back 3 months). No lock: each participant is settled by whichever process gets to them first |
| badge rarity | every `TOKULY_QUEST_RARITY_INTERVAL_SECONDS` | |
| pruning | every 6 hours | Missions past their claim window, per-day metric rows older than 70 days |

No job-queue jobs.

## Locks and connections

- No transaction of this domain asks the pool for a second connection:
  whatever a transaction needs from another domain (friends, summaries, the
  wallet) or from the pool is read before it opens, and wallet movements
  happen after it committed. One account sending many requests at once can
  therefore use connections, but never hold one while waiting for another.
- Everything that changes a player's numbers locks their `quest_profiles`
  row first (the flush, claims, rerolls, the draw, the showcase and title, a
  season result), so those changes happen one after the other. A streak
  locks its pair's row, then the two profiles (lower user ID first), then a
  restore order. Nothing takes these in another order.
- The decisions that must hold under concurrency are made by the database:
  a conditional update or a unique key decides who claims a mission, who
  uses the day's reroll, who gets the bonus, who places a month's restore,
  who sends a reminder and who settles a season result.

## Valkey keys

All under `quest:<namespace>:` (see `TOKULY_QUEST_KEYSPACE`).

| Key | Type, TTL | Purpose |
|---|---|---|
| `day:<user>:<date>` | hash, 3 days | A player's local day so far: `x:<source>` XP (never above the cap), `m:<metric>` counts, `h` local hours with activity, `v` version |
| `seen:<event>:<rule>:<user>` | marker, 48 h | A deed that was applied |
| `dirty` | sorted set | Days with changes not yet in PostgreSQL, scored by when to write them |
| `flush:<user>` | lock, 30 s | One process writes a player at a time |
| `tw:<user>:<friend>:<date>` | marker, 3 days / 10 min | The player's part of that streak day towards that friend is recorded / the two are not friends |
| `drawn:<user>:<date>` | marker, 2 days | The day's missions exist |

The day hashes hold absolute values and PostgreSQL only takes the difference
to what it has, so losing Valkey loses at most the seconds not yet written,
and writing a day twice changes nothing.

Rate limits (`rl:` keys, per user): `ClaimMission` 60/min, `RerollMission`
20/min, `RestoreStreak` 10/min, `GetLeaderboard` and `GetSeason` 120/min
together, `SetShowcase`, `SetTitle` and `UpdatePreferences` 30/min each, and
300/min for looking things up (`GetProfile`, `ListUserBadges` of somebody
else, `GetBadge`, `ListBadgeCatalog`; 120/min per address for guests).

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_QUEST_FLUSH_DELAY_MS` | `3000` | How long earnings are collected before they are written and pushed (100–60000). |
| `TOKULY_QUEST_LEVEL_UP_COINS` | `20` | Coins per level-up. |
| `TOKULY_QUEST_MILESTONE_LEVEL_COINS` | `100` | Coins for every tenth level instead. |
| `TOKULY_QUEST_STREAK_RESTORE_COINS` | `100` | Price of restoring a streak. |
| `TOKULY_QUEST_STREAK_GRACE_MINUTES` | `10` | Minutes after a midnight before a running streak is declared lost. |
| `TOKULY_QUEST_STREAK_REMINDER_MIN_DAYS` | `3` | Shortest streak a player is reminded about. |
| `TOKULY_QUEST_LIVE_RULE_DAYS` | `14` | A rule is live while it fired within this many days. |
| `TOKULY_QUEST_RARITY_INTERVAL_SECONDS` | `3600` | Pause between rarity recomputations. |
| `TOKULY_QUEST_KEYSPACE` | derived from the database URL and the NATS prefix | Namespace of the quest keys in Valkey, so deployments sharing a Valkey do not share a write queue. |

## For the server crate

```rust
let quest = Quest::new(core.clone(), identity.clone(), cfg.quest)?;   // after social and wallet
router = quest.register(router);
quest.spawn_workers();                                                 // worker role
```

`QuestConfig` is flattened into `DomainConfigs` like the others; migration
`1900` needs `0100`, `0200` and `1400`. No ClickHouse table of its own.

## Requests

- **notify**: a `quest` preference category (kind prefix `quest.`), so
  players can switch streak reminders and badge pushes off in the
  notification settings; today they fall into `system`, whose inbox toggle
  is locked on.
- **chat**: a lookup of a direct conversation's two members (for example
  `tokuly_chat::…::direct_members(core, conversation_id)`), wired to
  `Quest::set_direct_chats` in `bridges.rs`. Without it, messages count
  towards a streak only in conversations opened while this domain was
  listening (it learns them from `chat.conversation.created`; a new consumer
  starts with the 7 days the stream keeps). Naming the recipient in the
  props of `chat.message.send` would do as well.
- **geo**: footprint and visit events should not be emitted while a player
  is in ghost mode (this domain can see the `footprints_visible_to` setting,
  not ghost mode), or should say so in the payload.
- **geo, school, plans, spaces, community, live, shorts**: the expected
  topics above. Where a domain already emits something else for the same
  thing, change the row in `rules.rs` rather than adding an event.
