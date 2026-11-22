# spaces — events, jobs and configuration

Crate `tokuly-spaces`, migration `2200_spaces.sql`, API `tokuly.spaces.v1`
(`SpaceService`, `HangoutService`, realtime `SpacesEvent`). Audio runs on
LiveKit through `tokuly_core::livekit` (`core.livekit`, settings
`TOKULY_LIVEKIT_URL`, `_PUBLIC_URL`, `_API_KEY`, `_API_SECRET`). Without
them the RPCs that need audio answer `AUDIO_NOT_CONFIGURED` (unavailable)
and everything else works.

## Events emitted

Payload structs live in `tokuly_spaces::events`; each is written in the
transaction of the change it describes.

| Topic | Payload | When |
|---|---|---|
| `spaces.room.started` | `{ space_id, host_id, title, tags, language, audience, scheduled }` | A room went live (created on the spot, or a scheduled one started). Actor: the host. |
| `spaces.room.ended` | `{ space_id, host_id, audience, reason, minutes, peak_listeners, total_listeners }` | A live room is over. `reason` is `SpaceEndReason` as its number; `minutes` rounds up. Actor: the (last) host. |
| `spaces.room.joined` | `{ space_id, user_id, host_id, audience }` | Someone entered a room for the first time (not on re-entering; not the host starting it). Actor: the person. |
| `spaces.hangout.joined` | `{ hangout_id, user_id, first, inside_count }` | Someone entered a hangout's voice room; `first` when they opened it. Actor: the person. |

`audience` is `SpaceAudience` as its number (1 public, 2 friends, 3 invited).

## Events consumed

Durable consumer `spaces`. Every handler gives the same result when its
event arrives twice; the two about a state that can be undone look at the
current state, so late news of a lifted suspension or block does nothing.

| Topic | Reaction |
|---|---|
| `spaces.room.started` (own) | Fan-out: `spaces.reminder` to everyone who asked (reminders are consumed as they are sent), `spaces.started` to the host's friends for a friends room — once per room, and at most once per host every `TOKULY_SPACES_STARTED_ANNOUNCE_SECONDS`. Nobody else hears of a public room. |
| `identity.user.deleted` | The account's live room ends, its rooms and hangouts are deleted (a hangout with other members passes to the longest-standing one), every trace in other rooms and hangouts goes. |
| `identity.user.banned` | While the suspension is in force: the person leaves whatever they are inside, their live room ends and their upcoming rooms are cancelled (`ACCOUNT_CLOSED`, shown to others as `HOST_LEFT`). |
| `social.user.blocked` | In what the blocker runs, the blocked person is removed (room, invitation, kept co-host role, hangout). In what the blocked person runs, the blocker just leaves — nothing else changes there, so the block stays silent. Open hangout invitations between the two go. Reminders are left alone and never fire. |
| `admin.content.removed` | `SUBJECT_TYPE_SPACE`: a live room ends (`MODERATION`), an upcoming one is cancelled, a hangout is closed. |

## Realtime (`ServerFrame.spaces`)

Topic `space:<space id>`, guarded: whoever may see the room may follow it
(public rooms: anyone, also without signing in).

| Event | To | When |
|---|---|---|
| `space_updated` | topic | started, ended, handed over, title / tags / audience / schedule / pin changed |
| `participant_updated` | topic | someone came on stage, changed role, raised or lowered a hand |
| `participant_left` | topic (stage and raised hands), and the person when it was not their doing | left, removed, banned, dropped off, room ended |
| `counts` | topic | head count; every change up to 150 people, then at most every 2 s |
| `reaction` | topic | emoji burst (not stored; at most 300 per room and 10 s) |
| `speaker_muted` | topic | a moderator muted a speaker |
| `stage_invitation` | user | invited on stage / invitation withdrawn |
| `space_invitation` | user | invited to a room |
| `hangout_updated`, `hangout_removed`, `hangout_invitation` | members / invitees / user | the group changed |
| `hangout_presence` | members | someone entered or left the voice room |

## Notification kinds

| Kind | Recipient | Notes |
|---|---|---|
| `spaces.started` | friends of the host (friends rooms only) | actor: host; collapsed per room |
| `spaces.reminder` | who asked to be reminded; the host at the planned time (no actor) | collapsed per room |
| `spaces.invited` | invitee of a room or hangout | actor: inviter; at most 3 a day per inviter → invitee, later invitations arrive silently; `data.space_id` or `data.hangout_id` |
| `spaces.hangout_joined` | members who are friends of the first person inside and did not mute the hangout | actor: that person; collapsed per hangout; at most once per `TOKULY_SPACES_HANGOUT_ANNOUNCE_SECONDS`; never urgent |

The notify domain files unknown prefixes under `system`; a `spaces` category
would be a change there.

## Other outputs

- Activity entry `spaces.started` (public audience) when a public room starts.
- Search documents `kind: "space"` for public rooms that are upcoming or live; removed when they end, are cancelled or stop being public.
- `SubjectResolver` for `SUBJECT_TYPE_SPACE` (rooms and hangouts). Resolves for whoever may see the room and for everyone who was ever inside or invited — also after a removal, ban, block or deletion — so it stays reportable.
- Profile card `spaces.in_room` for friends who could enter the same room or hangout.
- `DataExporter` section `spaces`.

## HTTP

`Spaces::http_routes()` → `POST /spaces/livekit/webhook`: LiveKit's webhook,
verified with `core.livekit.verify_webhook`. Any event about one of this
domain's LiveKit rooms (`tokuly-space-<id>`, `tokuly-hangout-<id>`) triggers
a comparison of that venue with LiveKit; the event itself is never trusted,
so duplicates and reordering are harmless. LiveKit configuration:
`webhook: { api_key: <key>, urls: [https://<api>/spaces/livekit/webhook] }`.

## Background work

No job-queue jobs. `Spaces::spawn_workers` starts:

| Worker | Schedule | What it does |
|---|---|---|
| event consumer | on events | see above |
| `reconcile` then `settle_rooms` | `TOKULY_SPACES_RECONCILE_INTERVAL_SECONDS` | Compares live rooms, open hangouts, and rooms / hangouts that just ended or emptied with LiveKit: removes people who are not connected (after the connect / reconnect grace), cuts connections of people who are not inside, repairs permissions. Then hands rooms whose host stayed away to a co-host or ends them, and ends empty rooms. |
| `sweep` | 60 s | Host nudge at the planned time, cancels rooms nobody started, closes LiveKit rooms left open, purges old records. |

## Rate limits (`rl:spaces-*`)

Per person: create room 20/h, go live 6/h, update 30/min, enter 30/min,
hand 10/min, moderation actions 120/min, room invitations 30 calls/h,
pin 20/min, reminders 60/min, lists 120/min (per address when anonymous),
create hangout 10/day, hangout invitations 60 calls/h, hangout changes
60/min, emoji bursts `TOKULY_SPACES_REACTIONS_PER_10S`. Per pair: stage
invitations 4/min per moderator → listener; invitation announcements 3/day
per inviter → invitee. Per room: 300 emoji bursts and 100 announced hands
per 10 s (the rest is thinned out).

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_SPACES_MAX_SPEAKERS` | `10` | Seats on stage, host included. |
| `TOKULY_SPACES_MAX_COHOSTS` | `2` | Co-hosts per room. |
| `TOKULY_SPACES_MAX_LISTENERS` | `3000` | People per room. |
| `TOKULY_SPACES_HOST_GRACE_SECONDS` | `180` | How long a room waits for its host. |
| `TOKULY_SPACES_EMPTY_GRACE_SECONDS` | `120` | How long a room may stay empty. |
| `TOKULY_SPACES_CONNECT_GRACE_SECONDS` | `45` | Time to connect after entering. |
| `TOKULY_SPACES_RECONNECT_GRACE_SECONDS` | `20` | Time to reconnect after dropping off. |
| `TOKULY_SPACES_RECONCILE_INTERVAL_SECONDS` | `20` | Pause between comparisons with LiveKit. |
| `TOKULY_SPACES_TOKEN_TTL_SECONDS` | `600` | Lifetime of access tokens. |
| `TOKULY_SPACES_MAX_SCHEDULE_DAYS` | `30` | How far ahead a room can be scheduled. |
| `TOKULY_SPACES_MAX_SCHEDULED` | `10` | Upcoming rooms per host. |
| `TOKULY_SPACES_NEVER_STARTED_HOURS` | `24` | When an unstarted room is cancelled. |
| `TOKULY_SPACES_RETENTION_DAYS` | `90` | Finished rooms are kept this long after they ended. |
| `TOKULY_SPACES_EVIDENCE_DAYS` | `30` | Deleted rooms, closed hangouts and former members stay on record this long after the deletion. |
| `TOKULY_SPACES_STARTED_ANNOUNCE_SECONDS` | `1800` | Pause between two "friend started a room" fan-outs of one host. |
| `TOKULY_SPACES_REACTIONS_PER_10S` | `20` | Emoji bursts per person and room. |
| `TOKULY_SPACES_MAX_HANGOUTS` | `10` | Hangouts one person may own. |
| `TOKULY_SPACES_MAX_HANGOUT_MEMBERS` | `50` | People per hangout, invitations included. |
| `TOKULY_SPACES_MAX_HANGOUT_MEMBERSHIPS` | `100` | Hangouts one person may belong to. |
| `TOKULY_SPACES_HANGOUT_ANNOUNCE_SECONDS` | `600` | Pause between two "someone is inside" announcements of a hangout. |

## Public Rust API

```rust
Spaces::new(core, identity, cfg) -> anyhow::Result<Spaces>
Spaces::with_livekit(core, identity, cfg, livekit) -> anyhow::Result<Spaces>   // tools and tests
spaces.register(router) -> connectrpc::Router
spaces.spawn_workers()
spaces.http_routes() -> axum::Router
spaces.friends_in_rooms(viewer) -> AppResult<Vec<FriendInRoom>>   // friend lists, the map
spaces.handle_event(&event), reconcile(), reconcile_venue(venue), settle_rooms(), sweep()
```

## Schema notes

- Every change to a room or hangout happens under `FOR UPDATE` on its row; entering anything first takes a per-person advisory lock, so a person is inside one venue at a time.
- `space_participants` rows stay after people leave (`left_at`, `leave_reason`, `top_role`); `space_invites.revoked_at`, `spaces.deleted_at`, `hangouts.deleted_at` and `hangout_members.state = 3` hide things without erasing them.
