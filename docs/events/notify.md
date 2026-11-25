# notify — events, jobs and configuration

Crate `tokuly-notify`, migration `1200_notify.sql`, API `tokuly.notify.v1`
(`InboxService`, `PreferencesService`, `PushService`).

## How a notification travels

```
any domain ── outbound::notify(tx, NotificationRequest) ──▶ notify.requested
                                                               │
                              consumer "notify" (events::consume, worker processes)
                                                               │
        ┌──────────────────────────────┬───────────────────────┴───────────┐
   inbox entry                    realtime NotifyEvent                notify.push job
 (notifications table,       (the received event,                 (one per notification;
  collapsed by key)           only with an inbox entry)            fans out to devices)
```

Rules the consumer applies, in order:

1. Malformed requests (kind not `[a-z0-9_.]{1,64}`, empty title) and requests
   about the recipient's own action are dropped.
2. A recipient who muted the actor gets nothing: no inbox entry, no push.
   Nor does anyone in a block with the actor, whoever blocked whom (blocks
   are mirrored from `social.user.blocked` / `unblocked`; `system` notices
   are exempt). Blocking someone also removes what they caused from the
   blocker's inbox; the blocked person's inbox is left alone, since blocks
   are silent.
3. Category toggles: `inbox` off → no inbox entry (and no realtime event);
   `push` off → no push. The `system` category's inbox toggle is locked on.
4. Global push pause and quiet hours (in `users.time_zone`) suppress the push
   only. The inbox still records.
5. No push registration → no push job. More than
   `TOKULY_PUSH_RATE_PER_SENDER_PER_MINUTE` pushes caused by one person for
   one recipient in a minute, or more than `TOKULY_PUSH_RATE_PER_MINUTE`
   pushes for one user in a minute → the push is skipped, the inbox still
   records. The sender's budget is checked first, so one person's flood
   spends only their own and cannot silence everybody else's pushes.

**Urgent requests** (`NotificationRequest::urgent()`: incoming calls, SOS)
skip rule 4 and the recipient's per-minute cap of rule 5 (they have a
per-sender budget of their own, so nobody can ring through quiet hours
without end): they are pushed during quiet
hours and while pushes are paused. Rules 2 and 3 still apply — a muted person
and a category whose push is switched off stay silent. They are delivered at
the highest priority of every channel (table below), and the inbox entry
carries `urgent = true`.

**Withdrawn content.** Every inbox entry records the IDs it refers to: its
subject and every `*_id` value at the top level of its `data` (`comment_id`,
`message_id`...), and while it is unread, those of every event folded into
it. When one of them is deleted or taken down (events below), every entry
referring to it leaves every inbox (`NotifyEvent.removed` to the owners'
clients, in batches of 1000 entries). A grouped entry that still stands for
other events stays with one event less, and without its body and image when
it was showing the withdrawn one (clients see that change on their next
read; no event is sent for it). Content that is gone is remembered for
8 days: a request about it that arrives late, or a push job still queued for
it, is dropped. Content that is only hidden (a video back to draft) is
withdrawn from everyone but its owner and not remembered, since it can come
back. Content whose audience narrowed (a note narrowed to friends or made
private, a comment hidden by its subject's owner, by a block or held again;
see the events below) is withdrawn from everyone but its owner and those
the event names as still seeing it (`visible_to`), but only entries about
what happened up to the change (`changed_at`). The latest change per ID is
remembered for 8 days (`notify_audiences`): a change older than it does
nothing, a batch of a change stops once a newer change of the same ID was
recorded, and a request or a push job from before it is dropped for
everyone it left out. Entries of
the `system` category are never withdrawn: the notice telling an owner that
their content was taken down refers to that content. A withdrawal and a
request about the same ID serialize on an advisory lock, so an entry stored
while the withdrawal runs is not missed. Pushes already delivered cannot be
recalled.

**Edited comments.** Notifications of the engagement domain (`engagement.*`)
quote a comment (`data.comment_id`) at a version (`data.edited_at`, `null`
before the first edit). On `engagement.comment.edited` every entry quoting
an older version shows the body the event gives for the new text (and its
version), and an `engagement.mention` entry whose recipient the new text no
longer mentions leaves the inbox — except the subject owner's of a top-level
comment, which becomes the ordinary entry the event names (`owner_notice`:
engagement tells each person once, so a mentioned owner had only the
mention). Nobody the comment was hidden from (an audience change, below) is
shown the new text (`NotifyEvent.removed`; a changed body is
seen on the next read, no event is sent for it). The latest edit of each
comment is remembered for 8 days (`notify_edits`): an older edit handled
after a newer one changes nothing, and a request or a queued push about an
older version that is handled after the edit carries the new body — or is
dropped when it announces a mention that is gone (the owner's becomes their
ordinary entry).

The consumer is idempotent on the event ID (`notify_processed_events`, written
in the same transaction as the inbox entry). Events with the same collapse key
handled out of order: the older one is discarded.

Texts are stored in Japanese and English and picked when read: `users.locale`
for the inbox and realtime, the device's locale (falling back to the
account's) for push.

## Events consumed

| Topic | Payload | Effect |
|---|---|---|
| `notify.requested` | `tokuly_core::outbound::NotificationRequest` | inbox entry, realtime event, push job (see above) |
| `social.friend.prefs_changed` | `{ user_id, friend_id, muted }` (`tokuly_notify::events::FriendPrefsChanged`) | `muted = true` mutes `friend_id` for `user_id`, `false` unmutes. Applied only when newer than the pair's last change (from this event or `MuteUser` / `UnmuteUser`), so events handled out of order end in the latest state |
| `social.user.blocked` / `social.user.unblocked` | `{ user_id, blocked_id }` (`tokuly_notify::events::UserBlocked`) | mirrors the block (rule 2). A block removes `user_id`'s non-system entries caused by `blocked_id`. Applied only when newer than the pair's last change, so events handled out of order end in the latest state |
| `admin.content.removed` | `{ subject_type, subject_id, … }` | withdraws `subject_id` (any subject type but `SUBJECT_TYPE_USER`: a profile takedown keeps what the person did in other inboxes) |
| `chat.message.deleted` | `{ message_id, … }` | withdraws `message_id` |
| `engagement.comment.deleted` | `{ comment_id, … }` | withdraws `comment_id` |
| `engagement.comment.hidden` | `{ comment_id, author_id, visible_to, changed_at, … }` | withdraws `comment_id` (and so the replies below it, whose entries name it as `parent_id`) from everyone but `author_id` and `visible_to`, as far as the entries are not newer than `changed_at` |
| `engagement.comment.edited` | `{ comment_id, author_id, body, mentioned, owner_notice, edited_at, … }` | entries quoting an older version of the comment show `body`; mentions of people not in `mentioned` are withdrawn, the subject owner's turned into `owner_notice` (see "Edited comments") |
| `notes.note.deleted` / `notes.story.deleted` | `{ note_id, … }` / `{ story_id, … }` | withdraws the note / story |
| `community.thread.deleted` | `{ community_id, thread_id }` | withdraws `thread_id` |
| `video.playlist.deleted` | `{ playlist_id, owner_id }` | withdraws `playlist_id` |
| `video.video.deleted` | `VideoEvent` `{ video_id, … }` | withdraws `video_id` |
| `shorts.short.deleted` | `ShortEvent` `{ id, … }` | withdraws `id` |
| `live.stream.deleted` | `StreamEvent` `{ stream_id, … }` | withdraws `stream_id` |
| `ask.answer.deleted` | `{ question_id, owner_id }` | withdraws the asker's `ask.answered` entries for `question_id` (the question itself stays; not remembered, it can be answered again) |
| `video.video.unpublished` | `VideoEvent` | hides `video_id` from everyone but `owner_id` |
| `notes.note.visibility_changed` | `{ note_id, author_id, visibility, visible_to, changed_at }` | withdraws `note_id` (and what refers to it as its subject: likes, renotes, comments and mentions in comments on the note; likes on those comments name only the comment and stay) from everyone but `author_id` and `visible_to`, as far as the entries are not newer than `changed_at`; `visible_to = null` (public) withdraws nothing but is remembered, so an older narrowing handled after it does nothing. An event without `visible_to` withdraws only for `visibility` = 5 (private) |
| `video.video.visibility_changed`, `shorts.short.visibility_changed`, `live.stream.visibility_changed` | `{ video_id \| id \| stream_id, owner_id, visibility, visible_to, changed_at, … }` | the same for the video, short or stream: withdrawn from everyone but `owner_id` and `visible_to` up to `changed_at`; `visible_to = null` (open to everyone again) is remembered. Emitted when the owner shares it with another audience and, once per open item, when the owner locks or unlocks their account |
| `identity.user.created` | `{ user_id, … }` (`tokuly_notify::events::UserCreated`) | stores what the new account's pushes show: `PUSH_PREVIEW_SENDER_ONLY` for minors (by the birthdate, read this once), `PUSH_PREVIEW_FULL` for adults; never overrides a choice already made, so a redelivery changes nothing |
| `identity.user.deleted` | `{ user_id }` | deletes the user's inbox, preferences, mutes, mirrored blocks and push registrations, inbox entries they caused for others, mutes on them, the remembered edits of their comments and audience changes of their content, and their name in the remembered mentions and audiences of others' |

## Events emitted

| Topic | Payload | When |
|---|---|---|
| `notify.mute.changed` | `{ user_id, muted_user_id, muted }` (`tokuly_notify::events::MuteChanged`) | a mute was added or removed through `PreferencesService.MuteUser` / `UnmuteUser` (not when mirrored from `social.friend.prefs_changed`) |

The social domain can consume `notify.mute.changed` to keep a friend's
`muted` flag in step with mutes made from the notification settings.

## Notification kinds

Kinds are chosen by the producing domain. The segment before the first dot
selects the preference category:

| Category | Kind prefixes | Delivery priority |
|---|---|---|
| `social` | `social.`, `friend.` | high |
| `chat` | `chat.`, `call.` | high |
| `geo` | `geo.`, `map.`, `place.` | high |
| `notes` | `notes.`, `note.`, `story.`, `stories.` | normal |
| `video` | `video.`, `channel.`, `playlist.` | normal |
| `live` | `live.` | high |
| `spaces` | `spaces.`, `space.`, `hangout.` | high |
| `shorts` | `shorts.`, `short.` | normal |
| `engagement` | `engagement.`, `comment.`, `reaction.` | normal |
| `stamp` | `stamp.`, `stamps.`, `emoji.` | normal |
| `wallet` | `wallet.` | normal |
| `ask` | `ask.` | normal |
| `plans` | `plans.`, `plan.` | high |
| `school` | `school.`, `study.` | normal |
| `community` | `community.` | normal |
| `quest` | `quest.` | normal |
| `feed` | `feed.`, `news.`, `announcement.` | normal |
| `system` | `system.`, `identity.`, `account.`, `security.`, `admin.`, `media.`, `search.`, `dev.` and every unknown prefix | high |

How the three delivery priorities map to each channel:

| Priority | When | Web Push | FCM | APNs |
|---|---|---|---|---|
| normal | categories marked normal above | `Urgency: normal` | `android.priority: NORMAL` | `apns-priority: 5` |
| high | categories marked high above | `Urgency: high` | `android.priority: HIGH` | `apns-priority: 10` |
| urgent | the request is marked urgent, whatever its category | `Urgency: high`, `requireInteraction: true` | `android.priority: HIGH`, `notification_priority: PRIORITY_MAX` | `apns-priority: 10`, `aps.interruption-level: time-sensitive` |

"Normal" lets the push service wait for a moment that suits the device;
"high" wakes a sleeping device. The category only decides delivery speed: it
never gets a push past quiet hours or the pause. Urgent pushes also carry
`urgent` in their data (`notification.data.urgent`, FCM `data.urgent =
"true"`, APNs top-level `urgent`) so apps can present them differently, e.g.
a call screen.

Kinds this domain produces itself:

| Kind | When |
|---|---|
| `system.test` | `PushService.SendTestPush` (push only, ignores preferences) |

## Realtime (`NotifyEvent`)

Sent to every connection of the user:

- `received` — a new inbox entry, or a collapsed entry replaced by a newer
  event (same `id`), with the unread counts.
- `read` / `removed` — entries were marked read / deleted (possibly on another
  device). Lists the IDs, or sets `bulk` when more than 200 changed.

## Background work

| What | Name | Notes |
|---|---|---|
| Event consumer | durable `notify` | topics above |
| Push delivery job | `notify.push` (`tokuly_notify::delivery::PushJob`) | one job per notification; sends the current text of a comment edited while it waited (nothing for a mention the edit took out); dropped at delivery when its content was withdrawn, or when the recipient blocked, was blocked by or muted its actor in the meantime (not for `system` notices); up to 5 attempts with the job queue's backoff; devices already reached are remembered in Valkey (`notify:push:delivered:{job}`) so a retry only reaches the rest |
| Retention sweep | hourly timer | inbox entries older than `TOKULY_NOTIFY_RETENTION_DAYS`; handled-event markers, withdrawn-content markers, remembered comment edits and audience changes older than 8 days; push registrations of revoked or expired sessions |

Push outcomes: accepted → `last_delivered_at`; token gone (Web Push 404/410,
FCM `UNREGISTERED` / `SENDER_ID_MISMATCH`, APNs 410 / `BadDeviceToken` /
`DeviceTokenNotForTopic`) → registration deleted; 429/5xx/network → retried;
anything else → logged on the registration (`rejections`, `last_error`), not
retried.

Session revocation: identity publishes no event for it, so registrations are
checked against `sessions.revoked_at` / `expires_at` when a push is sent (a
revoked session's device gets nothing and its row is deleted) and swept hourly.

## Push payloads

### What pushes show (「通知に内容を表示」)

`Preferences.push_preview`, changed with `UpdatePreferences`:

| Choice | Title | Text | Picture, producer data | Tap opens |
|---|---|---|---|---|
| `PUSH_PREVIEW_FULL` 「すべて表示」 | as requested | as requested | kept | the notification's path |
| `PUSH_PREVIEW_SENDER_ONLY` 「送り主だけ」 | the category's name (「チャット・通話」) | 「あおいさんからメッセージが届いたよ」 (chat), 「あおいさんから動画のお知らせが届いたよ」 (others); without a nameable actor 「動画のお知らせが届いたよ」 | dropped | the notification's path |
| `PUSH_PREVIEW_HIDDEN` 「表示しない」 | `Tokuly` | 「新しいお知らせがあるよ」 | dropped | `/notifications` (the inbox) |

The name is the actor's display name (else handle) at delivery time;
producers leave the actor out of anonymous notifications, so those are never
named. The rewrite happens in `delivery.rs` on the per-device message
(`preview::apply`), before any channel builds its payload: what a choice
leaves out is not in the Web Push plaintext, the FCM request or the APNs
payload at all. The setting is read when the push is sent, so a change also
applies to pushes still queued. `kind`, `category` (the Android channel),
the collapse digest and the badge are kept. The inbox always shows
everything.

Default: decided when the account is created (`identity.user.created`) and
stored, `SENDER_ONLY` for accounts under 18 and `FULL` for the others; the
migration does the same for existing minors. It is never derived from the
birthdate on a read, so an 18th birthday changes nothing anyone can see.
When a push (or `GetPreferences`) finds nothing stored yet, because the
event has not been handled or was given up after its retries, it decides and
stores the default itself, the same idempotent way, so a minor's first push
never shows everything.

Collapse keys are sent to push services as a 32-character digest (Web Push
`Topic`, FCM `collapse_key` and `android.notification.tag`, APNs
`apns-collapse-id` and `thread-id`). The badge is the unread inbox count. The
click target is `core.web_url(path)`.

**Web Push** (RFC 8030 / 8291 / 8292, `aes128gcm`): the decrypted body is a
declarative push message, which the service worker can also read in its
`push` handler:

```json
{
  "web_push": 8030,
  "notification": {
    "title": "…", "body": "…", "lang": "ja",
    "navigate": "https://tokuly.example/chat/42",
    "tag": "<collapse key, else the notification id>",
    "image": "https://…",
    "timestamp": 1800000000000,
    "data": { "id": "<notification id>", "kind": "chat.message", "category": "chat", "path": "/chat/42", "extra": { } }
  },
  "app_badge": 3
}
```

**FCM** (`POST /v1/projects/{project}/messages:send`): a notification message
with `data` = `{ kind, category, path, url, badge, notification_id, data }`
(all strings; `data` is the producer's JSON, serialized), `android.notification.channel_id`
= the category.

**APNs** (`POST /3/device/{token}`, `apns-push-type: alert`): `aps.alert`,
`aps.badge`, `aps.sound`, `aps.thread-id`, `aps.mutable-content` (when there
is an image), plus top-level `kind`, `category`, `path`, `url`,
`notification_id`, `image_url`, `data`. `aps.interruption-level:
time-sensitive` takes effect only when the iOS app has the Time Sensitive
Notifications capability; without it iOS treats the notification as a normal
alert.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_VAPID_PRIVATE_KEY` | empty | Web Push application-server key (raw P-256 scalar, base64url). Empty derives it from `TOKULY_SECRET` (`keys.derive("webpush/vapid")`) |
| `TOKULY_VAPID_SUBJECT` | empty | Contact URI in VAPID tokens. Empty: the public URL when https, else `mailto:` + the `TOKULY_MAIL_FROM` address |
| `TOKULY_WEBPUSH_ALLOWED_HOSTS` | `fcm.googleapis.com,updates.push.services.mozilla.com,notify.windows.com,push.apple.com` | Hosts (and their subdomains) a Web Push endpoint may point to; `*` = any public https host. Whatever the list, Web Push requests only connect to public addresses: a name resolving to a loopback, private or link-local address fails |
| `TOKULY_WEBPUSH_ALLOW_INSECURE` | `false` | Development/tests: accept `http://` endpoints on any host. Startup fails if set in production |
| `TOKULY_FCM_SERVICE_ACCOUNT_JSON` | empty | Firebase service-account JSON; empty disables FCM |
| `TOKULY_APNS_KEY_P8`, `TOKULY_APNS_KEY_ID`, `TOKULY_APNS_TEAM_ID`, `TOKULY_APNS_TOPIC` | empty | APNs token-auth key (`.p8` content, `\n` escapes accepted), key ID, team ID, bundle ID; all four enable APNs |
| `TOKULY_PUSH_TTL_SECONDS` | `86400` | How long push services keep a notification for an offline device |
| `TOKULY_PUSH_RATE_PER_MINUTE` | `30` | Pushes per user per minute; 0 = no cap |
| `TOKULY_PUSH_RATE_PER_SENDER_PER_MINUTE` | `10` | Pushes one person may cause for one recipient per minute, and urgent pushes likewise; 0 = no cap |
| `TOKULY_NOTIFY_RETENTION_DAYS` | `90` | Days an inbox entry is kept |

FCM and APNs are implemented against the documented wire formats and tested
against local stand-ins only; they have not been run against Google's or
Apple's servers.
