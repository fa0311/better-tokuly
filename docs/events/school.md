# school — events, jobs and configuration

Crate `tokuly-school`, migration `2100_school.sql`, API `tokuly.school.v1`
(`SchoolService`, `TimetableService`, `StudyService`, `StudyRoomService`,
`ExamService`, `AssignmentService`, realtime `SchoolEvent`).

## How time is kept

- **The server keeps the clock.** Clients never report elapsed time. A timer
  is a position on a timeline of work and break phases; `TimerSnapshot`
  carries the phase, when it ends and the server's time, so a client only
  corrects for its own clock offset.
- **Study time is counted into segments** (`school_study_segments`) whenever
  the client is heard from (`HeartbeatStudySession`, pause, resume, stop...)
  and when a study room's timer changes for everyone in it. A session the
  client ends counts up to that moment. A session the server has to end —
  the client fell silent for `TOKULY_SCHOOL_SESSION_IDLE_SECONDS`, a pause
  outlasted its limit, the room closed, the host removed the member — is cut
  back to the client's last sign of life.
- **Days are the user's.** Every "per day" figure (totals, goal, streak,
  heatmap, countdowns, today's classes, urgency) uses `users.time_zone`
  (Asia/Tokyo unless changed). Totals are sums over segments cut at local
  midnights, so a session across midnight counts towards both days. The
  weekly ranking uses the viewer's week for everyone in it.

## Events emitted

Payload structs live in `tokuly_school::events`. All are written in the same
transaction as the change they describe; the actor is the user (the host for
room events).

| Topic | Payload | When |
|---|---|---|
| `school.study.session_ended` | `{ user_id, session_id, minutes, seconds, subject, subject_id, source, room_id, started_at, ended_at, manual, auto_stopped }` | A session of at least one minute entered the log: the timer was stopped by the user or the server, a stay in a study room ended (leaving, changing subject, room closed), or time was logged by hand (`manual: true`). `minutes` is whole minutes of counted study time (pauses and pomodoro breaks excluded); `subject` is the subject's name, empty when none; `source` is `tokuly.school.v1.SessionSource` as its number. Not emitted again when a logged session is edited. |
| `school.study.goal_reached` | `{ user_id, date, minutes, goal_minutes, streak_days }` | The session that just entered the log took the user's study time for `date` (their time zone) from below their daily goal to at or above it. At most once per day per user; only for users with a goal. |
| `school.room.opened` | `{ room_id, host_id, audience, title }` | A study room was created. `audience` is `tokuly.school.v1.RoomAudience` as its number. |
| `school.room.closed` | `{ room_id, host_id, reason }` | A study room closed. `reason` is `tokuly.school.v1.RoomCloseReason` as its number: 1 the host closed it, 2 it stood empty, 3 it was taken down or its host's account is gone. |

Suggested consumers: quest (study missions, study-time leaderboards) from
`session_ended` — it carries `manual` so hand-logged time can be capped —
and `goal_reached`; feed gets its entries directly (below).

## Events consumed

Durable consumer `school`. Every handler is idempotent
(`School::handle_event`).

| Topic | Reaction |
|---|---|
| `identity.user.deleted` `{ user_id }` | The user's open rooms close (reason 3; members' sessions end and are logged), their own session ends, and every row about them is deleted: settings, subjects, terms with grid / changes / overrides / shares, sessions with segments, cheers sent and received, rooms, invitations, bans, exams, assignments. Other people's terms copied from theirs keep their content and lose the reference. |
| `social.friend.removed` `{ user_id, friend_id }` | Each leaves the other's chosen timetable audiences, invitations between them are withdrawn, and neither stays inside a non-public room the other hosts. |
| `social.user.blocked` `{ user_id, blocked_id }` | Everything `social.friend.removed` does, plus: neither stays inside any room the other hosts, and the blocked user leaves rooms the two are in together. |
| `admin.content.removed` `{ subject_type, subject_id }` | `SUBJECT_TYPE_STUDY_ROOM`: the room closes (reason 3). Other types and unknown IDs are ignored. |

Friendship and blocks are also checked at read time (`tokuly_social::graph`),
so a timetable, a "studying" card or a room disappears for the other person
immediately; the events only clean up stored rows and people already inside
rooms.

## Realtime pushes

`ServerFrame.school` (`SchoolEvent`).

User-scoped (every device of the user):

| Event | Sent to | When |
|---|---|---|
| `session` | the user | Their session changed: started, paused, resumed, phase skipped, re-labelled, ended (also by the server), entered a room. |
| `friend_studying` | friends of a user who shows their studying | They started, paused, resumed or stopped; or switched `study_visibility` (hiding sends `studying: false`). |
| `cheer` | the user cheered on | `CheerStudy`. |
| `room_invitation` | each new invitee | `InviteToStudyRoom` / invitations at creation; carries the room as the invitee sees it. |
| `room_left` | a member who is out of a room without having left | Idle, removed (host, block, unfriending), room closed; carries the ended session. |

Topic `study:{room_id}` — guarded: only signed-in people who may enter the
room can subscribe (`TopicGuard` registered for the `study` kind):

| Event | When |
|---|---|
| `room_timer` | The host started / paused / resumed / skipped / reset the timer, its lengths changed, a phase ended by itself (announced by the sweep), or the timer started or paused by itself (host left, room emptied, someone entered an empty room). A `TimerSnapshot` with `server_time`. |
| `room_member_joined`, `room_member_left` (`reason`, `member_count`), `room_member_updated` | Presence: someone came in, went (left / idle / removed), or changed what they study. |
| `room_reaction` | One of the fixed reactions, to the room or to one member. |
| `room_updated` | Title, audience or member cap changed (no member list). |
| `room_closed` | The room closed (`reason`). |

## Notification kinds

Requested through `tokuly_core::outbound::notify`.

| Kind | Recipient | Channels | Notes |
|---|---|---|---|
| `school.class_soon` | the user | inbox + push | Opt-in (`SchoolSettings.class_soon_minutes`, 0 = off, the default). Sent that many minutes before the first class of the day that takes place (cancelled classes do not count), at most once per day. Collapsed (`school.class_soon`). `data = { date, period, subject, room, start_minute }`. No actor. |
| `school.assignment_due` | the user | inbox + push | Opt-out (`SchoolSettings.assignment_reminders`, per assignment `remind`). Two per open assignment: at `TOKULY_SCHOOL_ASSIGNMENT_EVENING_HOUR` o'clock on the day before the due date (user's time zone), and one hour before it is due. A moment that had passed when the assignment was saved is skipped; a reminder more than 6 hours late or past the due time is dropped. Collapsed per assignment. `data = { assignment_id, due_at, reminder: "evening" \| "hour" }`. No actor. |
| `school.cheer` | the user cheered on | inbox + push | Opt-in (`SchoolSettings.cheer_notifications`, off by default: a cheer is otherwise only the realtime event). Actor: the friend. Collapsed per session and sender. |
| `school.room_invite` | each new invitee | inbox + push | Actor: the host; subject: the room (`SUBJECT_TYPE_STUDY_ROOM`); collapsed per room. |

The notify domain has no `school` category yet, so these kinds currently fall
into its `system` category (see "Requests" below).

## Activity entries

Through `tokuly_core::outbound::activity`:

| Kind | Who | Audience | Notes |
|---|---|---|---|
| `school.studied` | the user | friends | A timed session (own timer or study room) of at least 10 minutes entered the log. Only when the user shares study activity (`SchoolSettings.share_study_activity`, default on) and activity at all (identity's `share_activity`). Not for time logged by hand. `data = { session_id, minutes, subject, room_id }`, path `/school/study`. |

## Profile cards

`School::new` registers a `tokuly_identity::ProfileExtension`. All three
cards are for the owner and their friends at most; a stranger's or anonymous
view gets none.

| Kind | Order | Shown when | Content |
|---|---|---|---|
| `school.studying` | 20 | The owner has a live session and `study_visibility` is friends. | Title 勉強中 (休憩中 while paused or in a pomodoro break), text `<subject> · <n>分`; `data = { session_id, state, resting, subject, studied_seconds, started_at, room_id, room_title }` (room only when the viewer may enter it); path `/school/study` or the room. |
| `school.now` | 30 | The viewer is in the audience of the owner's term for today, and a class is in progress or still to come today. | Title 授業中 / 次の授業, text `<n>限 <subject>（〜H:MM）` / `<n>限 <subject> H:MM〜`; `data = { state: "current" \| "next", period, subject, room, starts_at, ends_at }`. |
| `school.exam` | 40 | An exam with `show_on_profile` is on, or starts within 14 days. | Title テスト期間, text `<name> · あとN日` / `<name> · テスト期間中（あとN日）`; `data = { status, name, days_until_start, days_left, start_date, end_date }`; a path only for the owner. |

The map can show `school.now` and `school.studying` from the profile cards;
there is no separate call.

## Other hooks

- `SubjectResolver` for `SUBJECT_TYPE_STUDY_ROOM`: a room resolves for
  signed-in people who may enter it (owner = host, path
  `/school/rooms/{id}`, `interactions_enabled = false`: rooms can be
  reported, not commented on).
- `DataExporter` named `school`: settings, subjects, terms with grid,
  changes, overrides and shares, timetables shared with the user, study
  sessions and segments, cheers sent and received, rooms hosted,
  invitations, exams with results, assignments.

## Public Rust API

Other domains are expected to use the events above. For the server and for
tests:

```rust
pub fn new(core: Core, identity: Identity, cfg: SchoolConfig) -> anyhow::Result<School>;
pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router;
pub fn spawn_workers(&self);
pub async fn handle_event(&self, event: &tokuly_core::events::Event) -> AppResult<()>;
pub async fn sweep(&self, at: Time) -> SweepReport;                    // one pass of everything below
pub async fn announce_room_phases(&self, at: Time) -> AppResult<usize>;
pub async fn end_overdue_sessions(&self, at: Time) -> AppResult<usize>;
pub async fn close_empty_rooms(&self, at: Time) -> AppResult<usize>;
pub async fn send_class_reminders(&self, at: Time) -> AppResult<usize>;
pub async fn send_assignment_reminders(&self, at: Time) -> AppResult<usize>;
pub fn now(&self) -> Time;                                             // the domain's clock (ms precision)
```

`School::set_clock` / `shift_clock` (hidden from the docs) stop and move one
instance's clock; tests use them instead of waiting. Nothing in the server
calls them.

## Background work

No job-queue jobs. `School::spawn_workers` starts:

| Worker | Schedule | What it does |
|---|---|---|
| event consumer | on events | See "Events consumed". |
| sweep (`School::sweep`) | every `TOKULY_SCHOOL_SWEEP_INTERVAL_SECONDS` | (1) Pushes `room_timer` for rooms whose phase ended by itself and notes when the next one ends. (2) Ends sessions that are overdue — silent longer than the idle time, paused longer than the pause limit, or as long as a session may last — at their last heartbeat; members of rooms leave them (`IDLE`). (3) Closes rooms that stood empty for `TOKULY_SCHOOL_ROOM_EMPTY_CLOSE_SECONDS`. (4) Sends the class reminders that are due and schedules each user's next one. (5) Sends the assignment reminders that are due. Every step claims rows with row locks (`SKIP LOCKED` where it matters), so several worker processes can run it. |

The limits are also applied when the user's own client shows up again (any
session RPC), so they hold even where no worker runs.

## Analytics

`school.study.session` (subject `study_session`, value = counted seconds,
props `{ source, room_id, auto_stopped }`) for every logged session of at
least a minute. No ClickHouse table of its own.

## Rate limits (`rl:` keys)

Writes to school data 240/min per user; reads of other people's timetables
120/min; free-period search 30/min; timetable copies 20/h; statistics and
presence reads 240/min; session starts 30/min; manual log entries 60/h;
cheers 60/min per sender and 1 per 5 minutes per sender→recipient; room
creation 10/h; room joins 30/min; timer control 60/min; reactions 30/min;
invitations 60/h.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_SCHOOL_SESSION_IDLE_SECONDS` | `900` | Silence after which the server ends a session (and a stay in a room). Time is only ever counted up to the last heartbeat. |
| `TOKULY_SCHOOL_HEARTBEAT_INTERVAL_SECONDS` | `60` | How often clients are asked to send a heartbeat (at most half the idle time). |
| `TOKULY_SCHOOL_SESSION_MAX_PAUSE_SECONDS` | `10800` | How long a session may stay paused. |
| `TOKULY_SCHOOL_SESSION_MAX_SECONDS` | `43200` | Longest a single session may last (at most a day). |
| `TOKULY_SCHOOL_ROOM_EMPTY_CLOSE_SECONDS` | `600` | How long an empty room stays open. |
| `TOKULY_SCHOOL_ROOM_DEFAULT_MEMBER_CAP` | `20` | Member cap of a room created without one (2-50). |
| `TOKULY_SCHOOL_ASSIGNMENT_EVENING_HOUR` | `19` | Local hour of the "due tomorrow" reminder. |
| `TOKULY_SCHOOL_SWEEP_INTERVAL_SECONDS` | `5` | Pause between sweep passes. |

No new infrastructure.

## Schema notes

- A study room's members are the active sessions pointing at it
  (`school_study_sessions.room_id`, one active session per user), so presence
  and counted time are one fact. `school_rooms.member_count` is kept in step
  under the room's row lock. Every room operation locks the room first and
  member sessions after it, and never two rooms.
- A grid slot (weekday, period) holds one class for every week, or one for
  week A and / or one for week B; writes replace whatever they collide with.
- A deleted subject leaves the timetables; if sessions, exams or assignments
  use it, it is archived instead of erased.
- Japanese public holidays are computed (`timetable/holidays.rs`, the law as
  of 2020, equinox formulas valid until 2099); a term can ignore them
  (`observe_public_holidays`), and a class-day override beats them.

## Requests (shared files this domain could not edit)

- **server**: add `tokuly-school` to `crates/server` — `SchoolConfig` in
  `DomainConfigs`, `School::new(core.clone(), identity.clone(), cfg.school)`
  after social, `register`, `spawn_workers`. No bridge is needed.
- **notify**: a `school` preference category for the `school.` kind prefix
  (normal priority), so reminders and cheers can be switched off as a group
  instead of landing in `system`.
