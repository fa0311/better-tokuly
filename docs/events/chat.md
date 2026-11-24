# chat — events, jobs and configuration

Crate `tokuly-chat`, migration `0600_chat.sql`, API `tokuly.chat.v1`
(`ConversationService`, `MessageService`, `PovService`, `CallService`,
`AssistService`, realtime `ChatEvent`). Depends on `identity`, `social`,
`stamp` and `media`.

## API for other domains

```rust
// Get or create the direct conversation of two users for a flow the server
// starts itself (a thank-you after a gift, the demo simulator). No privacy
// audience is consulted and nobody has to accept a message request; a request
// that was waiting is accepted, a declined one stays declined.
Chat::open_direct(&self, a: Uuid, b: Uuid) -> AppResult<Uuid>            // conversation ID

// Post a text message as a member. Goes through the normal pipeline (unread
// counts, realtime, notifications, link preview) but skips request allowances
// and rate limits. Fails with CONVERSATION_NOT_FOUND (not a member),
// MESSAGE_EMPTY, TEXT_TOO_LONG, CONVERSATION_READ_ONLY (block, deleted account).
Chat::post_text(&self, conversation_id: Uuid, sender: Uuid, text: &str) -> AppResult<Uuid>   // message ID

// Post a notice from the server into a conversation: a system message
// (SYSTEM_EVENT_TYPE_NOTICE, the text in `SystemEvent.notice`) with no
// sender. It moves no unread counter, brings no hidden conversation back,
// requests no notification and cannot be reported or reacted to; it is
// pushed as `message_created`. Posted once per `key` (stored as the
// message's `client_id`, checked under the conversation lock), and at most
// NOTICES_PER_CONVERSATION_PER_DAY (5) a day per conversation. Returns
// false when it was there already or dropped by that limit. Fails with
// CONVERSATION_NOT_FOUND, NOTICE_KEY_INVALID (empty, > 100 characters),
// NOTICE_INVALID (no Japanese text, > 300 characters per language).
// Used by the developer platform when a bot changes hands.
Chat::post_notice(&self, conversation_id: Uuid, key: &str, notice: Localized) -> AppResult<bool>
tokuly_chat::direct_conversations_of(core, user_id) -> AppResult<Vec<Uuid>>    // where they are a member, any state

// The two people of a direct conversation (still members or not); None for
// groups and unknown conversations. `chat.message.created` names the
// conversation, not the recipient: this is how a consumer learns who a
// direct message went to (the server bridges it to the quest domain).
tokuly_chat::direct_members(core, conversation_id) -> AppResult<Option<(Uuid, Uuid)>>
```

Also public, as worker entry points and for tests: `Chat::handle_event`,
`handle_typing`, `handle_conversation_presence`, `sweep_presence`,
`resolve_link_preview`, `sweep_link_previews`, `expire_povs`,
`expire_ringing_calls`, `reconcile_calls`, `capabilities`, `livekit`.

## Events emitted

Payload structs live in `tokuly_chat::events`. All are written in the
transaction of the change they describe.

| Topic | Payload | When |
|---|---|---|
| `chat.conversation.created` | `{ conversation_id, kind, member_ids }` | A direct conversation was opened or a group created. `kind` is `ConversationKind` as its number. Actor: who did it (none for `Chat::open_direct`). |
| `chat.member.joined` | `{ conversation_id, user_id, added_by }` | A user was added to a group (also once per initial member of a new group). |
| `chat.member.left` | `{ conversation_id, user_id, removed_by }` | A user left a group (`removed_by` null), was removed, or lost the membership with their account. |
| `chat.message.deleted` | `{ message_id, conversation_id, sender_id, deleted_by, moderated }` | A message was deleted for everyone: by its sender, a group manager above them (the owner: anyone; an admin: plain members), moderation (`moderated: true`) or because its last upload was taken down (`deleted_by` null). |
| `chat.call.started` | `{ call_id, conversation_id, started_by, video }` | A call began ringing. |
| `chat.call.ended` | `{ call_id, conversation_id, end_reason, duration_seconds, participant_ids }` | A call ended. `end_reason` is `CallEndReason` as its number; `participant_ids` are the users who were connected at some point. |

Sending a message emits **no** domain event (it happens far too often);
messages are counted through analytics (below).

## Events consumed

Durable consumer `chat`. Every handler is idempotent.

| Topic | Reaction |
|---|---|
| `identity.user.deleted` `{ user_id }` | The user's memberships, reactions and co-presence go. Their messages stay (the sender renders as a deleted user). Direct conversations become read-only for the other side (a live call fails, an open POV request is cancelled). Groups lose a member, with the usual system message; a group they owned passes to the longest standing admin, else member; a group nobody is left in is deleted with its uploads released. |
| `social.user.blocked` / `social.user.unblocked` `{ user_id, blocked_id }` | The direct conversation of the two becomes read-only for both (live call ends as `FAILED`, open POV request is cancelled), or writable again. Sending already checks blocks live, so the event only maintains the flag lists show. Groups are not touched. |
| `social.friend.accepted` `{ user_id, friend_id, … }` | A message request waiting between the two is accepted (friends need none). |
| `media.asset.ready` / `media.asset.failed` `{ asset_id, … }` | Messages showing the upload (a video sent while transcoding) get the final attachment and are pushed again (`MESSAGE_UPDATE_REASON_ATTACHMENT`). |
| `media.asset.deleted` `{ asset_id, … }` | A forced deletion (takedown): the upload leaves every message showing it; a media message left with neither attachments nor caption becomes a tombstone; a group using it as icon loses the icon. |
| `admin.content.removed` `{ subject_type, subject_id, reason, moderator_id }` | `SUBJECT_TYPE_CHAT_MESSAGE`: the message is deleted for everyone (tombstone, `chat.message.deleted` with `moderated: true`). Other subject types are ignored. |

## Bus subjects (core NATS)

| Subject | Queue group | Purpose |
|---|---|---|
| `sys.typing` (`tokuly_core::presence::TYPING_SUBJECT`) | `chat-typing` | Typing signals forwarded by the realtime gateway. Dropped unless the user is an accepted member of a conversation that can be written in; fanned out as `ChatEvent.typing` to the other accepted members (not to members in a block with the typist). |
| `sys.conversation_presence` (`CONVERSATION_PRESENCE_SUBJECT`) | `chat-presence` | "Opened / left the conversation screen". Dropped for non-members. Recorded in Valkey; changes are fanned out as `ChatEvent.presence` (the complete set) to the accepted members. |

## Co-presence (Valkey)

| Key | Type | Content |
|---|---|---|
| `chat:present:<conversation>` | sorted set | user IDs scored with the unix time their presence runs out (`PRESENCE_TTL_SECONDS` = 75 s after the last signal; clients repeat it every 30 s). Entries past their score never count, so a crashed client disappears by itself. |
| `chat:present:due:<nats prefix>` | sorted set | `<conversation>:<user>` by the same expiry, for the sweeper that tells the others when someone went away silently. |

Everyone with the conversation open suppresses their own message
notifications; only accepted members who share their online status
(`share_online_status`) are *shown* as present. Other keys: `chat:translation:<message>:<language>:<text hash>`
(cached translations, 24 h), `chat:reaction-notified:<message>:<user>` (a reaction was notified, 24 h), `chat:reaction-pair:<author>:<user>` (5 min) and the rate limits below (`rl:chat-…`).

## Realtime pushes

`ChatEvent`, always user-scoped: to every device of the
members concerned, the actor's other devices included. Members who declined a
conversation receive nothing from it.

| Event | Sent to | When |
|---|---|---|
| `message_created` | members who can see the message | Any new message (user, system, call, POV). The quote above a reply is withheld from members who cannot see the answered message. |
| `message_updated` | members who can see the message | Edited, deleted (tombstone), link preview arrived, attachment finished/failed/removed, pinned, unpinned, call state changed, POV state changed. |
| `reaction_changed` | members | A reaction was added or removed (with the new count of that emoji). |
| `read_receipt` | accepted members | A read cursor moved forward (also the sender's, when sending skipped unread messages). A member reading a message *request* only tells their own devices. |
| `typing` | other accepted members | See bus subjects. |
| `presence` | accepted members, each without the members in a block with them (and nothing at all to someone in a block with whoever changed) | The set of members shown as present changed (signal, leaving the group, expiry). |
| `conversation_updated` | all members (shared fields, no `viewer`, no `last_message`) or one member (with `viewer`) | Rename, icon, members, read-only; or the recipient's own state: added to a group, role, request accepted, preferences changed on another device. |
| `conversation_removed` | the member | They left, were removed, hid the conversation or declined the request. |
| `call` | accepted members | A call started ringing, was answered, gained or lost a participant, or ended. |

## Notification kinds

Requested through `tokuly_core::outbound::notify`; the actor is always the
other user (the notify domain applies per-person mutes). Path `/chat/<conversation>`.

| Kind | Recipient | Channels | Notes |
|---|---|---|---|
| `chat.message` | other members | push only | Collapsed per conversation (`chat:<conversation>`). Not sent to members who muted the conversation, who have it open right now (co-presence), who are in a block with the sender (groups), or to a pending/declined message request. Subject `(SUBJECT_TYPE_CHAT_MESSAGE, message)`, `data = { conversation_id, message_id, seq }`, image for media. |
| `chat.mention` | mentioned members | push only | Same collapse key. A mention by `@handle` gets through a conversation mute; `@all` does not. |
| `chat.request` | recipient of a message request | inbox + push | Once per request (its first message), not for the following ones. Does not quote the message: the inbox entry outlives an unsend or a takedown. |
| `chat.reaction` | author of the message | push only | Collapsed per message (`chat-reaction:<message>`); not when muted, present, or blocked. Once per reacting person and message within 24 h (`chat:reaction-notified:<message>:<user>`), so switching a reaction off and on does not push again, and at most once per reacting person and author every 5 minutes, whatever message (`chat:reaction-pair:<author>:<user>`). |
| `chat.group` | users added to a group | inbox + push | Also for the initial members of a new group. |
| `chat.call` | callees | push only, **urgent** while ringing | Collapsed per call (`chat-call:<call>`). Direct calls and groups of up to 8 members ring (urgent, `data.ringing = true`); larger groups get an ordinary, non-urgent push. Group members who muted the conversation get neither this nor the missed-call notice. A missed call replaces the ringing push with a non-urgent inbox + push notification (`data.missed = true`). |
| `chat.pov` | the other side of a POV request | inbox + push | `data.event` = `requested` / `answered` / `saved`; collapsed per request (`chat-pov:<request>`). Not when muted or present. |

## Background work

| Worker | Interval | What |
|---|---|---|
| event consumer `chat` | — | See "Events consumed". |
| typing / co-presence fan-out | — | See "Bus subjects". |
| job `chat.link_preview` (`LinkPreviewJob { message_id }`) | — | Fetches the preview of a message's first link through `tokuly_media::link_preview` (at most 30 per sender and minute; more links simply get none), stores it and pushes `message_updated` (`LINK_PREVIEW`). Queued when a message with a link is sent or edited. 8 in parallel, 3 attempts. |
| co-presence sweep | 10 s | Announces members whose presence ran out without a goodbye. |
| call ringing time-out | 5 s | After `TOKULY_CHAT_RING_SECONDS`: an unanswered direct call ends as `MISSED`; in a group the members who did not answer are marked missed (and notified) while the call stays open. |
| call reconciliation | 30 s | Lists the participants of each live call's LiveKit room (calls older than a minute); someone marked as joined but not connected 60 s after joining (`CONNECT_GRACE_SECONDS`) has left, which may end the call, a participant still marked as joined who is no longer an accepted member has left, and anyone connected who is not a joined participant (hung up, removed from the group, back with an old token) is put out of the room. Skipped while LiveKit is unreachable. LiveKit webhooks are not used. |
| POV expiry | 60 s | Open requests past their 2 h become `EXPIRED`; unsaved answers past their 24 h lose their content (attachments and caption removed, uploads released). |
| link preview sweep | 5 min | Re-queues links nobody looked up for 2 minutes (a lost job) and gives up on those older than a day. |
| removed text purge | 1 h | Forgets what edited and unsent messages said (`chat_removed_texts`, kept for reports) 30 days after the edit or deletion. |

## Media assets

| Holder | Asset | Attached | Detached |
|---|---|---|---|
| `chat:message:<message_id>` | every upload a message shows (`ASSET_PURPOSE_CHAT_IMAGE/VIDEO/AUDIO/FILE`); forwards hold the original sender's upload under their own message | when the message is stored | when the message is deleted for everyone, when a POV answer expires, when its group is deleted |
| `chat:icon:<conversation_id>` | a group's icon (`ASSET_PURPOSE_AVATAR`, image) | `CreateGroup` / `UpdateGroup` | when the icon is replaced or removed, when the group is deleted |

Attachments are stored in the message as `tokuly.media.v1.Attachment` JSON;
`chat_message_assets` indexes them (gallery, lookups by asset). Download
links of private files are renewed on every read.

## Subjects

| Subject type | Resolves when | Owner | Notes |
|---|---|---|---|
| `SUBJECT_TYPE_CHAT_MESSAGE` | the viewer is a member of the conversation and can see the message (not from before they joined or cleared their history; not declined) and it is not deleted. **For a report** (`resolve_for_report`) a message its sender (or a group manager) deleted still resolves for 30 days after the deletion | the sender (nil UUID for system messages; a notice from `Chat::post_notice` does not resolve at all: nobody is behind it) | For reports only (`interactions_enabled` is false). Path `/chat/<conversation>?message=<id>`; title is the current text (or a description of the content). For a report of a message that was edited or deleted: `[N版] <v1> → <v2> → … → <current or last text>`, each version cut to 80 characters. Anonymous viewers and non-members never resolve it. |

Unsending or editing must not take away the recipients' way to report what
they received. So the text a message had before each edit (the first 20
edits; later ones keep nothing and the report title says so), and what it
said when its sender (or a group manager) deleted it, is kept in
`chat_removed_texts` for 30 days from that change. Only the report path of
the subject resolver reads it: bookmarks, previews and everything else see
the message as it is now. A takedown (moderation, a removed upload) deletes
every kept version (the report it acted on has the text). The sender's data
export lists their kept versions (`removed_texts`).

## Analytics events

| Name | Subject | Value / props |
|---|---|---|
| `chat.message.send` | `("conversation", id)`, user = sender | `{ kind, group, forwarded }` — text, stamp, media, location and hello messages |
| `chat.call.end` | `("conversation", id)`, user = caller | value = duration in seconds; `{ video, end_reason, participants }` |

Best-friend ranking (`tokuly_social::graph::record_interaction`): `Message`
/ `Stamp` per message — to the other side of a direct conversation; in a
group to the people a message addresses (the answered sender, `@mentions`,
at most 10). `Call` in minutes for every pair of participants of a finished
call who were connected (the shorter of the two stays; the 12 longest-staying
participants of a group call).

## LiveKit rooms

A call's room is `tokuly-call-<call id>`; it is created when the call
starts and deleted when it ends (everyone is disconnected). Join tokens
(`JoinCall`, `StartCall`) grant joining that one room, publishing and
subscribing, and must be used within 2 minutes (`JOIN_TOKEN_TTL`). A token
cannot be revoked, so whoever leaves a call that goes on — hanging up,
leaving or being removed from the group, a deleted account — is also
removed from the room (`RemoveParticipant`), and the reconciliation puts
out anyone who connects again without being a joined participant.

## Data export

Exporter `chat`: the user's conversations (with role and preferences), the
messages they sent, their reactions, the calls they took part in and their
POV requests.

## Configuration

| Variable (flag) | Default | Meaning |
|---|---|---|
| `TOKULY_LIVEKIT_URL` (`--chat-livekit-url`) | empty | LiveKit server API (Twirp), e.g. `http://livekit:7880`. |
| `TOKULY_LIVEKIT_PUBLIC_URL` (`--chat-livekit-public-url`) | empty | What clients connect to (`ws://…` / `wss://…`). |
| `TOKULY_LIVEKIT_API_KEY` / `TOKULY_LIVEKIT_API_SECRET` (`--chat-livekit-api-key` / `-secret`) | empty | Credentials tokens are signed with. Calls need all four LiveKit settings; with none of them every call RPC that needs the media server fails with `CALLS_DISABLED`. Setting only some is a startup error. |
| `TOKULY_CHAT_RING_SECONDS` (`--chat-ring-seconds`) | `45` | How long an unanswered call rings (5–300). |
| `TOKULY_ANTHROPIC_API_KEY` (`--chat-anthropic-api-key`) | empty | Enables the AI assistant. **When it is used, message text leaves the server** (Anthropic Messages API). |
| `TOKULY_ANTHROPIC_BASE_URL` (`--chat-anthropic-base-url`) | `https://api.anthropic.com` | Base URL of the Messages API (tests point it at a stub). |
| `TOKULY_ANTHROPIC_VERSION` (`--chat-anthropic-version`) | `2023-06-01` | `anthropic-version` header. |
| `TOKULY_AI_MODEL` (`--chat-ai-model`) | `claude-opus-5-5` | Model of the assistant. |
| `TOKULY_AI_EFFORT` (`--chat-ai-effort`) | `low` | `output_config.effort`; empty omits it (for models without effort levels). |
| `TOKULY_AI_REFUSAL_FALLBACK` (`--chat-ai-refusal-fallback`) | `true` | Sends `fallbacks: "default"` with the beta header `server-side-fallback-2026-07-01`, so a request the safety classifiers decline is retried by the API on the recommended fallback model. Switch off for models or platforms without server-side fallbacks. |
| `TOKULY_AI_HOURLY_LIMIT` (`--chat-ai-hourly-limit`) | `60` | Assistant requests per user and hour. |
| `TOKULY_AI_TIMEOUT_SECONDS` (`--chat-ai-timeout-seconds`) | `45` | Longest wait for one answer, retries included. |
| `TOKULY_CHAT_MAX_GROUP_MEMBERS` (`--chat-max-group-members`) | `100` | Members per group. |
| `TOKULY_CHAT_EDIT_WINDOW_SECONDS` (`--chat-edit-window-seconds`) | `900` | How long after sending a message can be edited. |
| `TOKULY_CHAT_REQUEST_MESSAGE_LIMIT` (`--chat-request-message-limit`) | `3` | Messages a non-friend may send before their request is accepted. |
| `TOKULY_CHAT_MESSAGES_PER_MINUTE` (`--chat-messages-per-minute`) | `120` | Messages one user may send per minute. |
| `TOKULY_CHAT_POV_ANSWER_SECONDS` (`--chat-pov-answer-seconds`) | `7200` | How long a POV request can be answered. |
| `TOKULY_CHAT_POV_REPLY_SECONDS` (`--chat-pov-reply-seconds`) | `86400` | How long an unsaved POV answer lives. |

Fixed limits: 4000 characters per message, 10 uploads per message, 50
mentions, 50-character group names, 20 pinned conversations per user, 50
pinned messages per conversation, 20 different emoji per message, 10
messages × 10 conversations per forward, 200-character POV notes, 4 uploads
per POV answer, 2000-character drafts for the assistant.

### AI assistant: what is sent

`POST {base}/v1/messages` with `x-api-key`, `anthropic-version` and a JSON
schema for the answer (`output_config.format`); no sampling parameters, and
thinking is left to the model's default.

| RPC | Sent | Never sent |
|---|---|---|
| `RewriteDraft` | the draft and the chosen tone | who wrote it, for whom |
| `SuggestReplies` | the text (≤ 500 characters each) of the last ≤ 12 text messages of the conversation, labelled `me:` / `other:` | names, handles, IDs, attachments, deleted messages, POV answers |
| `TranslateMessage` | the text of that one message and the target language | sender, conversation |

Nothing is sent when a request is invalid, rate limited, answered from the
translation cache, or when there is nothing to reply to. Transient failures
(408/409/429/5xx, connection errors) are retried twice; a refusal
(`stop_reason: "refusal"`) is `AI_REFUSED`.

## Rate limits

| Key | Limit |
|---|---|
| `chat-send:<user>` | `TOKULY_CHAT_MESSAGES_PER_MINUTE` / minute (`SendMessage`, `SendHello`, `EditMessage`, each forwarded copy) |
| `chat-open:<user>` | 60 new direct conversations / hour |
| `chat-group-create:<user>` | 20 / hour |
| `chat-add-members:<user>` | 60 / hour |
| `chat-group-update:<user>` | 60 / hour (`UpdateGroup`) |
| `chat-role:<user>` | 60 / hour (`SetMemberRole`) |
| `chat-pin:<user>` | 30 / minute (`PinMessage`) |
| `chat-react:<user>` | 120 / minute |
| `chat-search:<user>` | 60 / minute |
| `chat-pov:<user>` | 30 / hour |
| `chat-notice:<conversation>` | 5 / day (`Chat::post_notice`; silently dropped beyond) |
| `chat-call:<user>` | 20 / 10 minutes |
| `chat-join:<user>` | 60 / 10 minutes (`JoinCall`) |
| `chat-ai:<user>` | `TOKULY_AI_HOURLY_LIMIT` / hour |
| `chat-link:<sender>` | 30 link previews / minute (silently none beyond) |
| `chat-typing:<user>`, `chat-presence:<user>` | 120 signals / minute each (silently dropped beyond) |
