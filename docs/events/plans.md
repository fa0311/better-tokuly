# plans — events, jobs and configuration

Crate `tokuly-plans`, migration `2000_plans.sql`, API `tokuly.plans.v1`
(`FreeService`, `PlanService`, `PollService`, `AnniversaryService`,
`CalendarService`, realtime `PlansEvent`). Depends on `core`, `proto`,
`identity` and `social`.

Everything in this domain is among friends; nothing is public, nothing is
indexed for search.

## Events emitted

Payload structs live in `tokuly_plans::events`. All are written in the same
transaction as the change they describe. None carries user-written text or a
position: notes, areas and titles stay inside the domain.

| Topic | Payload | When |
|---|---|---|
| `plans.free.set` | `{ user_id, status_id, window, starts_at, ends_at, audience }` | A user said "ひま" (also when replacing their status). `window` / `audience` are `FreeWindow` / `FreeAudience` as numbers. |
| `plans.plan.created` | `{ plan_id, host_id, invitee_ids, starts_at, poll_id }` | A plan was created, by `CreatePlan` or by finalizing a date poll (`poll_id` set). `starts_at` is null while the time is to be decided. |
| `plans.plan.cancelled` | `{ plan_id, host_id }` | The host called a plan off. |
| `plans.plan.rsvp_changed` | `{ plan_id, user_id, rsvp, previous }` | A participant's answer changed, including moving up from the waitlist. `rsvp` / `previous` are `Rsvp` as numbers. |
| `plans.plan.attended` | `{ plan_id, user_id, host_id, attendee_ids }` | The plan's time has passed and `user_id` was going. One event per person who was going; `attendee_ids` lists all of them. Only for plans that existed before they started (a plan written down after the fact, to split a bill, counts nobody), and once per plan: a plan that is moved to another day after it was counted is not counted again (`plans.attended_at`), so whatever other domains grant for attendance is granted once. |
| `plans.poll.created` | `{ poll_id, creator_id, invitee_ids, slot_count }` | A date poll was started. |
| `plans.poll.finalized` | `{ poll_id, creator_id, plan_id, answered_count }` | A date poll was turned into a plan (`plans.plan.created` is emitted alongside). |
| `plans.anniversary.started` | `{ anniversary_id, kind, user_ids: [lo, hi] }` | A "days since" counter came into being: a friendship counter was started, or a couple invitation was accepted. |

## Events consumed

Durable consumer `plans`. Every handler is idempotent
(`Plans::handle_event`) and judges by the state it finds, not by the order
of arrival: an event about a friendship that has been restored since changes
nothing, an event about a block that has been lifted since counts as the end
of the friendship, and an event about a row that is gone is not an error.

| Topic | Reaction |
|---|---|
| `identity.user.deleted` | Deletes the user's status, answers, alerts, countdowns, counters, polls, poll answers and settings, and takes them out of other people's chosen audiences. Plans they hosted pass to a co-host (else someone going, else anyone); a plan with nobody else in it is deleted. Their participation is deleted — except where they are part of a cost split: there the row stays as a declined guest without a comment, so the others' balances still add up (the account itself is an anonymous tombstone by then). |
| `social.friend.removed` | `{ user_id, friend_id, blocked }`: the two leave each other's audiences (chosen friends of a status or a countdown, "tell me when they are free", "あそぼ!" answers), unanswered plan invitations between them are withdrawn (one sent the other, or to a plan the other hosts), each leaves the other's polls with their answers, and their counters and couple invitations end. Plans they both answered keep both. |
| `social.user.blocked` | `{ user_id, blocked_id }`: exactly the same as the end of a friendship, so that nothing here tells a blocked person apart from an unfriended one (see "Blocks" below). |
| `admin.content.removed` | `{ subject_type, subject_id, … }` with `SUBJECT_TYPE_PLAN`: the plan disappears for everyone in it (`PLAN_NOT_FOUND`), its reminders are dropped. |

## Realtime pushes

`PlansEvent`, always user-scoped. Plan and poll events
carry IDs and counters, never names or comments, so the same event is right
for every participant.

| Event | Sent to | When |
|---|---|---|
| `free_status_changed` | friends in the status's audience | A status was set (with the status as friends see it) or ended early (`status` unset). People who saw the previous status and are not in the new audience get the "ended" form. Not sent when a window simply runs out (clients know `ends_at`). For the best-friends audience only the friends who asked to be told are pushed to, see "Best friends" below. |
| `free_answer` | the owner of the status | Someone answered "あそぼ!". |
| `plan_changed` | participants | `INVITED` to new invitees; `UPDATED`, `CANCELLED`, `PARTICIPANTS` to everyone in the plan; `REMOVED` to someone who left, was removed, lost a pending invitation, or whose plan was taken down. |
| `plan_rsvp_changed` | participants | An answer changed (one event per person, also for waitlist promotions), with the new counts. |
| `checklist_changed` | participants | An item was added, changed or removed. |
| `expenses_changed` | participants | An expense was added, changed or removed, or a transfer was marked as done / taken back. |
| `poll_changed` | participants | Live tallies after every answer; status changes; `details_changed` when texts, slots, deadline or participants changed; `deleted` when the poll is gone — also sent to someone who left it or was taken out of it. |
| `anniversary_changed` | the other person of a counter | The counter was started, invited to, accepted, changed or ended. Declining an invitation is not pushed to its sender. |

## Notification kinds

Requested through `tokuly_core::outbound::notify`. The notify domain files
unknown prefixes under its `system` category; a `plans` category
(`plans.`, high priority for `plans.nudge` and `plans.reminder`) would suit
these better.

| Kind | Recipient | Channels | Notes |
|---|---|---|---|
| `plans.nudge` | owner of a free status | inbox + push | "あそぼ!". Actor = sender; collapsed per sender (`plans.nudge:<sender>`); sent once per sender and status. |
| `plans.friend_free` | friends who opted in (`SetFreeAlert`) | inbox + push | A friend they asked about set a status they may see. At most once per window (a status overlapping the window already announced is not announced again) and at most once an hour per friend, however often the status is set. Collapsed per friend. |
| `plans.invited` | each invitee | inbox + push | Actor = whoever invited; collapsed per plan. |
| `plans.updated` | participants who have not declined | inbox + push | Only when the time or the place changed, which a plan allows 6 times an hour. Collapsed per plan. |
| `plans.cancelled` | participants who have not declined | inbox + push | Collapsed per plan. |
| `plans.rsvp` | the host | inbox + push | Actor = the participant; collapsed per plan (`plans.rsvp:<plan>`). |
| `plans.promoted` | someone who moved up from the waitlist | inbox + push | No actor. |
| `plans.reminder` | participants going or maybe | inbox + push | The day before and N minutes before the start, each once per start time: changing the lead time does not send one again, a new start time does. Actor = the host (none on the host's own copy); not sent to someone in a block with the host. Collapsed per plan. |
| `plans.settled` | the payer of a transfer | inbox + push | The receiver marked a transfer of the cost split as received. `data = { settlement_id, amount, currency }`. |
| `plans.settlement_removed` | the payer of a transfer | inbox + push | The receiver took that mark back: the debt is open again. Same `data`. |
| `plans.poll_invited` | each invitee | inbox + push | Collapsed per poll. |
| `plans.poll_answer` | the creator | inbox + push | First answer of each participant (taking it back and answering again is not announced again); collapsed per poll. |
| `plans.poll_reminder` | invited people who have not answered | inbox + push | Once automatically, and when the creator nudges (`RemindPollParticipants`); collapsed per poll. |
| `plans.poll_closed` | the creator | inbox + push | The poll closed at its deadline. No actor. |
| `plans.countdown` | the owner | inbox + push | "Today is the day", on the morning of the day in the owner's time zone. |
| `plans.counter_started` | the friend | inbox only | A friend started a friendship counter with them. |
| `plans.couple_invite` | the invited friend | inbox + push | Collapsed per sender. |
| `plans.couple_accepted` | the sender of the invitation | inbox + push | |
| `plans.milestone` | each of the two people of a counter | inbox + push | 100 / 200 / 300 / 500 days, every 1000 days, half a year, every anniversary — on the morning of the day in the recipient's own time zone, once per milestone. Actor = the other person. `data = { anniversary_id, milestone, date }` (`milestone` is `d100`, `half`, `y2`…). |

Notifications never go between two people in a block with each other
(plan changes, answers, settlements). Everything that notifies another person
is limited per sender, and invitations also per recipient: see "Limits".

## Activity entries

Through `tokuly_core::outbound::activity`. Both are `Private`: every person
gets an entry in their own log naming the others (`with_user_ids`), so
together they are visible to the participants and to nobody else.

| Kind | Who | Notes |
|---|---|---|
| `plans.created` | the host | subject `SUBJECT_TYPE_PLAN`, `with_user_ids` = invitees (up to 20), location of the place when it has one |
| `plans.attended` | everyone who was going, when the plan is over | subject `SUBJECT_TYPE_PLAN`, `with_user_ids` = the others who were going (up to 20), location of the place |

## Profile cards

`Plans::new` registers a `tokuly_identity::ProfileExtension`. Cards are shown
to the profile's owner and to their friends only.

| Kind | Shown | `data` |
|---|---|---|
| `plans.free` | while the owner has a status and the viewer is in its audience | `status_id`, `starts_at`, `ends_at`, `note`, `area_name` |
| `plans.anniversary` | a couple counter both partners chose to show (on both profiles); not to a viewer the partner is hidden from | `anniversary_id`, `partner_id`, `start_date`, `days` |
| `plans.countdown` | up to three countdowns the owner pinned, nearest first, until their day has passed | `countdown_id`, `date`, `days_left` |

## Other registrations

- `SubjectResolver` for `SUBJECT_TYPE_PLAN`: a plan resolves for its
  participants (owner = host, path `/plans/<id>`); interactions are off once
  it is cancelled; `comment_count` is kept on the plan. It also resolves,
  with interactions off and under the title they last saw, for people who
  *were* in it (`plan_past_participants`): removed by a host, gone by their
  own choice, or uninvited because a friendship ended or a block was set. So
  a plan stays reportable for everyone it was shown to — withdrawing an
  invitation, removing the invitee, renaming the plan, or being blocked by
  the invitee does not take the handle for a report away. A plan a moderator
  took down resolves for nobody.
- Date polls and "ひま" notes have no subject type of their own, so they can
  only be reported through their author's profile.
- `DataExporter` named `plans`.

## Blocks

Blocking is silent. Inside this domain a block does exactly what the end of
a friendship does (see the events above), and nothing more that the blocked
person could see:

- A plan shared by two people in a block keeps both, and both stay listed in
  `Plan.participants` like everyone else — name, answer and comment, the way
  a shared group shows its people. There is no marker. The same holds for a
  poll two friends of its creator share.
- No notification goes from one to the other (plan changes, answers,
  settlements), and a plan's reminders do not reach someone in a block with
  its host.
- Nobody is brought into a plan whose host they are in a block with: a
  co-host or guest who invites such a friend gets a normal answer in which
  that friend was simply not invited.
- Lists of things addressed to one person leave the other out
  (`ListFreeAnswers` and `FreeStatus.answer_count`, counters, birthdays).

## Accounts that are suspended or deleted

A suspended account's "ひま" status is not shown to anyone while the
suspension lasts. Its plans, polls and counters stay as they are: they are
shared with the other people in them. A deleted account is removed as
described under `identity.user.deleted`.

## What an answer gives away

Whatever the caller may not see is reported as not found, with the same
reason as something that does not exist: a status whose audience the caller
is not in, a countdown not shared with them, a poll they were not invited to,
a plan they are not in. An invitation to a couple counter whose sender has
become part of a couple in the meantime is withdrawn like any other
withdrawn invitation (`ANNIVERSARY_NOT_FOUND`); `ALREADY_IN_COUPLE` only ever
speaks about the caller.

## Best friends and friendship dates

`tokuly_social::graph` has no function for "is X one of Y's best friends" or
"since when are X and Y friends". What the social domain does offer is its
part of a profile (`ProfileExtension::enrich_profile` fills in
`Profile.best_friend` and `Profile.friends_since`), so `Plans::new` takes the
`Social` domain and `Plans::friendship` asks it for that part, on an empty
profile. Consequences:

- the best-friends audience can be checked one pair at a time (about eight
  queries) but not listed: such a status is found by `ListFreeFriends` /
  `GetFreeStatus` and the profile card, and pushed only to friends who opted
  in to alerts;
- when such a status is set, at most 100 of the friends who asked to be told
  (the most recent requests) are checked for being a best friend;
- each answer is remembered for 5 minutes (`plans:best-friend:<owner>:<friend>`),
  so starring or un-starring a friend can take that long to show.

A `graph::best_friend_ids(core, user_id)` and a
`graph::friends_since(core, a, b)` in the social crate would replace this,
and the `Social` argument of `Plans::new` with it.

## Birthdays

Birthdays and who sees them are identity's: `users.birthdate` and the
privacy setting `birthday_visible_to` / `birthday_show_year`, read through
`tokuly_identity::birthdays::shared_birthdays` (batched).
`AnniversaryService.UpdatePlansSettings` (`share_birthday`) is the switch on
this domain's screen and writes that setting (on: friends, or everyone when
it already was; off: nobody). Only the month and the day are returned here.

## Background work

No job-queue jobs. `Plans::spawn_workers` starts the event consumer above and
one timer that calls `Plans::run_due_work` every
`TOKULY_PLANS_WORKER_INTERVAL_SECONDS`. A pass does, in order:

| Step | What is due | What happens |
|---|---|---|
| free statuses | `ends_at` passed | the row is deleted |
| day-before reminders | `plans.day_before_due_at`: 24 hours before a start time; for a plan on a day, the evening before at `TOKULY_PLANS_DAY_BEFORE_HOUR` | `plans.reminder`, unless one was sent for this start time (`day_before_sent_for`) |
| "soon" reminders | `plans.before_due_at`: `remind_minutes_before` before a start time | `plans.reminder`, unless one was sent for this start time (`before_sent_for`) |
| attendance | `plans.attended_due_at`: the end (or 3 hours after a start without an end; the end of the day for a plan on a day) | `plans.plan.attended` + activity entries |
| poll deadlines | `date_polls.deadline` | the poll closes, `plans.poll_closed` |
| poll reminders | `date_polls.remind_due_at`: 24 hours before the deadline, or 48 hours after creation without one; once per poll (`reminded_at`), whatever the deadline is changed to | `plans.poll_reminder` |
| countdowns | `plan_countdowns.notify_due_at`: `TOKULY_PLANS_MORNING_HOUR` on the day, owner's time zone | `plans.countdown`; yearly ones are scheduled for next year |
| milestones | `plan_anniversaries.notice_due_at`: the earlier of the two people's mornings of the next milestone day | `plans.milestone` to whoever's day it is |

Rows are claimed with `FOR UPDATE SKIP LOCKED` in batches, so several worker
processes can run at once. A `*_due_at` column is cleared (or moved to the
next occurrence) in the same transaction that sends, and nothing is sent
late: a reminder that would come after the start, or a "today" notice after
the day, is dropped.

## Valkey keys

| Key | Type, TTL | Purpose |
|---|---|---|
| `plans:best-friend:<owner>:<friend>` | `1` / `0`, 5 min | Remembered answer to "is `friend` one of `owner`'s best friends". Dropped when the two stop being friends. |

## Limits

Rate limits (`rl:` keys in Valkey; refused with `RATE_LIMITED`). Per account
unless it says otherwise:

| What | Limit |
|---|---|
| set a free status | 20 / hour |
| "あそぼ!" | 30 / minute, 10 / minute to the same friend (one notification per sender and status) |
| turn a free alert on or off | 120 / hour |
| create a plan | 30 / hour |
| change or cancel a plan | 60 / hour |
| `InviteToPlan` / `InviteToPoll` calls | 60 / hour |
| invitations sent, every invitee of `CreatePlan`, `InviteToPlan`, `CreatePoll` and `InviteToPoll` counted | 300 / hour |
| invitations from one person to the same friend, plans and polls together | 10 / day |
| changes of a plan's time or place (each notifies everyone in it) | 6 / hour per plan |
| RSVP | 12 / minute and 60 / hour |
| remove a participant, leave, change a role | 300 / hour |
| checklist changes | 300 / hour |
| expense changes | 300 / hour |
| record or take back a transfer | 60 / hour |
| create a poll | 20 / hour |
| change, close or reopen a poll, take someone out of it | 120 / hour |
| answer a poll | 20 / minute (the creator is told once per participant) |
| nudge the people who have not answered | 1 per poll every 12 hours |
| create or change a countdown | 120 / hour |
| start a friendship counter | 20 / day, 5 / day with the same friend |
| invite to a couple counter | 10 / day, 3 / day to the same friend |
| change a counter | 120 / hour |

Caps: 100 people in a plan or a poll (configurable), 100 checklist items and
500 expenses per plan, 30 slots per poll, 100 countdowns (3 pinned) and 200
free alerts per account.

## The cost split cannot be used against someone

- An expense names only people who are going, as payer and as holders of a
  share (`PAYER_NOT_GOING`, `SHARE_NOT_GOING`): nobody is charged for a plan
  they declined or never answered, and nobody is kept in a plan
  (`HAS_EXPENSES`) by a share they did not agree to. Someone already part of
  an expense may stay in it when it is edited.
- A transfer is recorded by the person who received the money, never by the
  one who owes it, and only up to what is owed between the two: a debt
  cannot be declared paid by the debtor, and a payment cannot be invented by
  or about someone outside the split.
- Taking a transfer back (the receiver only) keeps the row with who did it
  and when (`GetSplitResponse.removed_settlements`) and notifies the payer.

## Leaving

- `LeavePlan` is for good: the person is not invited to that plan again
  (later invitations skip them without a notification).
- `LeavePoll` removes the caller and their answers; the creator cannot
  invite them again, but through the link of an open poll they can come back
  by themselves. `RemovePollParticipant` (creator) takes someone out; the
  link no longer lets them in until they are invited again.
- A poll takes no answers after its deadline, whether or not the worker has
  closed it yet, and its link can be switched off in any state.

## Requests at the same moment

- A request that holds a transaction uses that transaction's connection and
  no other: what has to be asked of the social or identity domain is asked
  before the transaction starts. So requests waiting for the same lock never
  hold connections the lock holder needs (`tests/concurrency.rs` calls every
  locking RPC more often at once than the pool has connections).
- Everything that changes a plan (participants, answers, checklist, expenses,
  transfers) runs under the plan's row lock; everything that changes a poll
  under the poll's. Two people cannot both take the last place, a poll
  becomes one plan however often it is finalized, and a transfer can only be
  recorded while that much is owed (`AMOUNT_EXCEEDS_BALANCE`), so a double
  tap records it once.
- Per-account state that has a limit or must be unique (the one free status,
  free alerts, countdowns and pins, counters between two people, the one
  couple) is serialized with transaction-scoped advisory locks.
- Worker passes claim rows with `SKIP LOCKED`; alerts for a free status are
  claimed in the transaction that stores it.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_PLANS_MAX_PARTICIPANTS` | `100` | Most people in one plan or one date poll, the host included. |
| `TOKULY_PLANS_FREE_NOW_HOURS` | `3` | How long a "free now" status lasts. |
| `TOKULY_PLANS_WORKER_INTERVAL_SECONDS` | `30` | Pause between worker passes. |
| `TOKULY_PLANS_DAY_BEFORE_HOUR` | `18` | Local hour of the day-before reminder for plans on a day without a time. |
| `TOKULY_PLANS_MORNING_HOUR` | `8` | Local hour from which milestones and countdowns are announced on their day. |

## Schema notes

- `free_statuses`: at most one row per user; replaced on every set.
- `plans` + `plan_participants` (host included, one row per person),
  `plan_past_participants` (who was in a plan and no longer is, with the
  title they last saw; only read by the subject resolver),
  `plan_checklist_items`, `plan_expenses` + `plan_expense_shares` (shares are
  computed when an expense is written and stored), `plan_settlements`.
- `date_polls`, `date_poll_slots`, `date_poll_participants` (the creator
  included), `date_poll_answers`, `date_poll_past_participants` (who left or
  was taken out).
- `plan_settlements` rows are never deleted by a user: taking a transfer
  back sets `removed_at` / `removed_by`.
- `plan_countdowns`, `plan_anniversaries` (one row per pair and kind,
  `user_lo < user_hi`), `plan_anniversary_notices` (milestones already
  announced), `plan_settings`.
- Money is `bigint` in the currency's minor unit; currencies never mix: the
  split is computed per currency.
