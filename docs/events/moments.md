# moments — events, jobs and configuration

Crate `tokuly-moments`, migrations `2500_moments.sql` and
`2501_moments_bands.sql`, API `tokuly.moments.v1`
(`MomentService`, `MomentReactionService`, realtime `MomentsEvent`).
Pictures are uploads of `ASSET_PURPOSE_MOMENT` (the pair) and
`ASSET_PURPOSE_REALMOJI` (selfie reactions).

## Days

A region is a band of UTC offsets (`rules::BANDS`): an account is in the
band of its `time_zone`'s offset right now (`moment_zones`, kept up to date
by the planner, daylight saving included).

| Region | Offsets | e.g. | Window (local, 9–22 h configured) |
|---|---|---|---|
| `utc-11..-7` | −11 to −7 | Hawaii, Alaska, US Pacific | Hawaii 10–19, Los Angeles (winter) 12–21 |
| `utc-6..-2` | −6 to −2 | US Central and Eastern, Brazil | New York (winter) 10–19, São Paulo 12–21 |
| `utc-1..+3` | −1 to +3 | Europe, Africa, Moscow, Saudi Arabia | London (winter) 10–19, Moscow 13–22 |
| `utc+3:30..+6:30` | +3:30 to +6:30 | Iran, Gulf, India, Bangladesh | Delhi 11–21 |
| `utc+7..+11` | +7 to +11 | South-east Asia, China, Japan, Korea, eastern Australia | Tokyo 11–20 |
| `utc+12..+14` | +12 to +14 | New Zealand, Fiji, Tonga, Kiribati | Auckland (winter) 9–20 |

Six, because one prompt moment must fall inside the local window of every
zone of its band: a band spans at most four hours of offsets, so each zone
keeps at least 9 of the 13 hours, and the world gets six prompts a day (one
every four hours or so) instead of one every few minutes somewhere. The
seams lie where few people live. Etc/GMT+12, west of every band, joins the
first.

A band's days (`moment_days`) are drawn a day ahead: one per date (the date
at the band's western edge), the prompt time uniform inside the part of
`[TOKULY_MOMENTS_PROMPT_FROM_HOUR, _UNTIL_HOUR)` that the whole band shares;
the first draw of a date stays. A date drawn in its middle gets what is left
of its window, or no prompt.

The prompt goes out once, through the job `moments.prompt`:

1. **Announce**, under the day's row lock: `announced_at` is set, the
   region's current day (`moment_regions.current_day_id`) moves to it and
   `moments.prompt.sent` is emitted, in one transaction. A day more than an
   hour past its time (workers were down), or overtaken by a later date that
   went out, is skipped instead and never starts.
2. **Tell**, 500 accounts per transaction in ID order: each account of the
   region (not bots) whose own day (`moment_people`) is of an earlier date
   moves to this day, and those of them that take prompts (not suspended,
   not too young) get one `moments.prompt` notification each, committed
   with the cursor `told_through`. A job that dies, runs twice or in two
   processes at once tells nobody twice; the planner queues the job again
   until `told_at` is set.

An account's **day** is its own: the latest prompt that reached it, until
the next one. Its date only moves forward, so a date brings one prompt and
one moment, wherever the time zone moves: after a change the account keeps
its day (late is late) and the new zone counts from the next date's prompt
that reaches it there. The one other way into a day is to post while the
region's prompt is still on its way (announced, the fan-out not past the
account yet): the post fixes the day, and the fan-out then tells nobody who
has it already. On time is up to `announced_at` +
`TOKULY_MOMENTS_WINDOW_SECONDS` (plus the upload grace); `late_seconds`
counts from the end of the window.

## Who sees what

| Viewer | Sees |
|---|---|
| the owner | every own moment, every day, in full, with the location |
| a friend (or close friend, by the moment's audience) | the owner's moment of the owner's day; never across a block, never of a suspended or deleted account; without the region (`MomentDay.region`, it says roughly where the owner is) |
| … who has no live moment of their own day | the same, **locked**: pictures as ThumbHash and colour only, no caption, location or reactions; a deleted own moment locks again, and so does a prompt on its way to the viewer |
| … unlocked | everything; the location only while they see the owner's precise live position (`tokuly_core::geoindex::sees_precisely`, for a list `seen_precisely` in one round trip: sharing on, no ghost rule hides it from them), judged now |
| anyone else | nothing (`MOMENT_NOT_FOUND`, as for a moment that does not exist) |

Reactions are listed without people on either side of a block with the
viewer and without suspended or deleted accounts. Unlocked moments shown to
someone other than the owner are recorded (`moment_deliveries`) for reports.

## Events emitted

Payload structs live in `tokuly_moments::events`; each is written in the
transaction of the change it describes. None carries a picture, caption or
location.

| Topic | Payload | When |
|---|---|---|
| `moments.prompt.sent` | `{ day_id, region, date, prompt_at }` | A region's day began. Once per region (a band, see "Days") and date. No actor. |
| `moments.moment.posted` | `{ moment_id, owner_id, day_id, date, late_seconds, audience }` | A moment was posted. `audience` is `MomentAudience` as its number. Actor: the owner. |
| `moments.moment.deleted` | `{ moment_id, owner_id, reason }` | Deleted: `owner`, `moderation` (actor: the moderator), `media_deleted`, `account_deleted` (no actor). |
| `subject.deleted` | `{ subject_type, subject_id }` | A moment (`SUBJECT_TYPE_MOMENT`) or reaction (`SUBJECT_TYPE_MOMENT_REACTION`) went for good: purged after the evidence period, or with an erased account. |

## Events consumed

Durable consumer `moments`. Every handler gives the same result when its
event arrives twice. Blocks, unfriendings and suspensions need none: every
read asks the social graph and the account's state.

| Topic | Reaction |
|---|---|
| `identity.user.created`, `identity.user.birthdate_given` `{ user_id, minor }` | The defaults of the age group, stored: the location on for adults, off for minors (no `minor`: a minor's). Never once the person set it themselves. An account without settings reads as a minor's. |
| `identity.user.deleted` | Its moments with every reaction on them, its reactions on others', its RealMoji library, settings and deliveries go at once, uploads released. |
| `admin.content.removed` | `SUBJECT_TYPE_MOMENT`: deleted (`moderation`), kept for reports like an owner's deletion. `SUBJECT_TYPE_MOMENT_REACTION`: taken down. |
| `media.asset.deleted` | A moment showing the upload is deleted (`media_deleted`), a reaction taken down, a library entry removed. |

## Realtime (`MomentsEvent`)

User-scoped only; nothing to subscribe to.

| Event | To | When |
|---|---|---|
| `prompt` (`MomentDay`) | each account told, with its page | the region's prompt reached it |
| `friend_posted` (`MomentRef`) | the moment's audience | a friend posted; clients fetch it to see it as they may |
| `deleted` (`MomentRef`) | the audience and the owner | a moment was deleted |
| `reactions_changed` | the owner | a reaction came, changed or went (the count the owner sees) |

## Notification kinds

| Kind | Recipient | Notes |
|---|---|---|
| `moments.prompt` | everyone of the region who takes prompts and has no day of that date | no actor; collapsed (`moments-prompt`) |
| `moments.reaction` | the moment's owner | actor: the reactor; collapsed per moment and reactor; at most 10 an hour per reactor → owner, later ones arrive silently |

The notify domain files both under its `moments` category ("いまの瞬間",
high delivery priority: the window is two minutes).

## Other outputs

- `SubjectResolver`s for `SUBJECT_TYPE_MOMENT` and `SUBJECT_TYPE_MOMENT_REACTION`, no comments or engagement reactions. For a report a moment resolves also after its deletion or its day, for its owner and everyone it was shown to unlocked; a reaction also after it was replaced or taken back, for the reactor, the moment's owner and everyone the moment was shown to. Until the purge.
- `DataExporter` section `moments`.
- Stamps sent as reactions are recorded with `SendContext::Comment`.
- Uploads are held as `moments:moment:<id>`, `moments:reaction:<id>` and `moments:realmoji:<user>:<emoji>`. A reaction holds the library picture it was made with, so changing the library changes nothing sent.

## Background work

`Moments::spawn_workers` starts:

| Worker | Schedule | What it does |
|---|---|---|
| event consumer | on events | see above |
| planner (`Moments::plan`) | `TOKULY_MOMENTS_WORKER_INTERVAL_SECONDS` | Maps every time zone to its band (every 10 minutes), draws today and tomorrow of each band, queues `moments.prompt` for every due day not fully told. |
| job `moments.prompt` | payload `PromptJob { day_id }`, concurrency 2, 20 attempts | `Moments::send_prompt`, see "Days". |
| purge (`Moments::purge`) | hourly | Deletes for good what was kept for reports past `TOKULY_MOMENTS_EVIDENCE_DAYS`: deleted moments with their reactions, replaced or taken back reactions; uploads released, `subject.deleted` emitted. |

## Rate limits (`rl:moments-*`)

Per person: post 10/h, delete 30/h, react or take a reaction back 60 per 10 min, RealMoji library
30/h, settings 30/h, reads 240/min. Per pair: reaction notifications 10/h
per reactor → owner.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_MOMENTS_WINDOW_SECONDS` | `120` | How long after the prompt a moment is on time. |
| `TOKULY_MOMENTS_UPLOAD_GRACE_SECONDS` | `60` | Extra time for the upload of a moment taken in the window. |
| `TOKULY_MOMENTS_PROMPT_FROM_HOUR` | `9` | Local hour from which the prompt may go out. |
| `TOKULY_MOMENTS_PROMPT_UNTIL_HOUR` | `22` | Local hour before which it has gone out; more than 4 hours (a band) after the first. |
| `TOKULY_MOMENTS_WORKER_INTERVAL_SECONDS` | `20` | Pause between planner passes. |
| `TOKULY_MOMENTS_EVIDENCE_DAYS` | `30` | How long deleted moments and replaced reactions stay on record, from the deletion. |

## Public Rust API

```rust
Moments::new(core, identity, cfg) -> anyhow::Result<Moments>
moments.register(router) -> connectrpc::Router
moments.spawn_workers()
moments.plan(), refresh_regions(), draw_days(now), due_days(), send_prompt(day_id) -> Sent
moments.handle_event(&event), erase_account(user_id), purge()
```

## Schema notes

- `UNIQUE (user_id, day_id)` makes one moment per day; a deleted one keeps its row (and the day) until the purge. One per date follows from `moment_people.date`, which only moves forward (conditional upsert).
- One live reaction per person and moment: a partial unique index, with a per-pair advisory lock around replacing it.
- `moment_days` and the region's current day change only under the day's row lock (`FOR NO KEY UPDATE`, so moments and accounts can refer to the day while it is told).
- Regions from before the bands (time zones) keep their past days and moments; nothing draws them any more.
