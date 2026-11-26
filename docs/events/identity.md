# identity — events, jobs and configuration

Crate `tokuly-identity`, migration `0100_identity.sql`, API `tokuly.identity.v1`
(`AuthService`, `AccountService`, `ProfileService`). Depends on `core`,
`proto` and `media`. Every other domain depends on it.

## Events emitted

All are written on the transaction of the change (the transactional outbox),
except `identity.export.ready`.

| Topic | Payload | When |
|---|---|---|
| `identity.user.created` | `{ user_id, handle, invite_code }` | A person signed up. Never for bots. |
| `identity.user.updated` | `{ user_id }` | The account's public face changed: profile fields, the verified badge, the プロフ帳. Search rebuilds the account's document on it. |
| `identity.profile.updated` | `{ user_id, changed: [group…] }` | The プロフ帳 changed. `changed` names the groups: `mbti`, `blood_type`, `pronouns`, `long_bio`, `theme`, `visibility`, `oshi`, `tags`, `links`, `qa_cards`, `pinned_song`. Carries no text: a consumer reads what it may show (audiences apply). Emitted with `identity.user.updated`. |
| `identity.user.status_changed` | `{ user_id }` | |
| `identity.user.privacy_changed` | `{ user_id }` | Privacy settings changed, including who sees the birthday (also when the plans domain's switch changed it). |
| `identity.user.deletion_requested` | `{ user_id }` | |
| `identity.user.email_verified` | `{ user_id }` | The account's address was confirmed (verification, change or undo of a change). |
| `identity.user.deleted` | `{ user_id }` | The account was erased: every domain removes that user's data. |
| `identity.user.banned` / `identity.user.unbanned` | `{ user_id }` | |
| `identity.export.ready` | `{ user_id, export_id }` | |

**No activity entries for profile edits.** "プロフィールを更新しました" in
friends' feeds would be noise (people edit the プロフ帳 in many small
saves), and an activity entry goes to friends as a whole while each field
has its own audience: announcing "changed their MBTI" to friends would leak
a change to a field only the owner sees. Domains that want to react
(search, feeds) follow `identity.profile.updated` and read the field under
its audience.

## Events consumed

Durable `identity-sessions`:

| Topic | Effect |
|---|---|
| `identity.user.banned` | Its own event: every session the account had when it was banned is revoked and its access tokens refused at once (`Identity::sign_out_banned`). Whoever banned signs the account out right after the commit too; this makes it at least once. A redelivery repeats the cache revocation of sessions revoked shortly before the ban and leaves sessions made after the ban alone. Parked sessions of other devices' account switchers are sessions like any other and end the same way. |

## Jobs and timers

| Name | What |
|---|---|
| `identity.export` | Builds the personal data archive (the account, the プロフ帳 with its audiences, settings, sessions with their switch marks, every domain's exporter) and mails a link. |
| `identity.profile.links` | `{ user_id }`: at most 20 never-checked links per account and day (the rest wait), at most 2 pages at once per host and process; fetches the account's profile links that are due (never checked, or checked a week ago) through media's guarded fetcher (`tokuly_core::egress`: public addresses only, every redirect hop checked, 512 KiB, 5 s) and marks a link verified while the page has an `<a>` or `<link>` with `rel="me"` pointing at `<public url>/u/<handle>` or `/@<handle>`. Queued (deduplicated per account) after links change, after a handle change, and by `RecheckProfileLinks`. Idempotent; an edit in between wins. |
| timer, 6 h | Queues `identity.profile.links` for accounts with links that are due. |
| timer, 1 h | Erases accounts whose deletion grace period ran out (the プロフ帳 and its uploads included) and emits `identity.user.deleted`. |

## Public Rust API for other domains

- `users::{summaries, summaries_ordered, summary, pick, filter_active, …}`, `settings::{privacy_of, privacy_of_many}` (unchanged).
- **Bots** (`bots`): `create_bot_account(tx, owner, handle, display_name, bio) -> Uuid`, `set_bot_owner(tx, bot, owner)`, `is_bot(core, id)`, `bots_among(core, ids) -> HashSet`, `bot_owners(core, ids) -> HashMap<bot, owner>`, and the one rule for places where only people count: `humans_only(core, ids) -> Vec<Uuid>` (or `NOT u.bot` in a join on `users`). Friend suggestions, leaderboards, rankings, nearby discovery, intro counts and anonymity group sizes use it.
- **Birthdays** (`birthdays`): `shared_birthdays(core, owners, closeness) -> HashMap<Uuid, SharedBirthday { month, day, year: Option }>` (batched, the caller says who is a friend), `Identity::visible_birthdays(viewer, owners)` (friendships and blocks from the profile extensions), `set_birthday_audience(core, user, audience, show_year)`, `audience_of(&PrivacySettings)`.
- **プロフ帳** (`extras`): `search_terms(core, ids, &locked) -> HashMap<Uuid, ProfileSearchTerms { text, tags }>` (public fields of unlocked accounts only, for search documents), `Extras::load(..).view(&Viewer)`, `reaches(visibility, circle, &Viewer)` (an allowlist: unknown values reach only the owner), `tag_key`.
- **Moderation**: `Identity::extras_for_moderation(user_id)` — the プロフ帳 as far as it reaches anybody, with its audiences (not "only me" fields or hidden cards). Not an RPC: the admin domain calls it from `ModerationService.GetReportedProfile` after checking that an open case is about the account and that the moderator is neither the account nor one of its reporters, and records `case.profile_viewed` in its audit log.
- **Audiences** come from the social domain through two `ProfileExtension` hooks, `audience_tie` (friend / follower / other, locked, close friend, circles) and `locked_among`, which the server wires to `tokuly_social::graph` (`bridges.rs`). Fields take PUBLIC, FOLLOWERS, FRIENDS, CLOSE_FRIENDS or PRIVATE; cards also a CIRCLE (`AudienceRef`). A locked account's PUBLIC parts reach its followers and friends only, and stay out of search documents and tag suggestions. Tags are suggested once 5 people show them.
- **Sessions**: `Identity::create_session_with(user, client, meta, SessionOrigin { mfa, added_by_switch })` beside `create_session`.

## Several accounts on one device

See the module documentation of `device_accounts.rs` for the model and the
threat model. In short: each account added with `add_account` keeps its own
session; the others are parked (`RefreshToken` answers `SWITCH_REQUIRED`
for a parked session); `SwitchAccount` activates one without its password,
asking for its TOTP when it has two-factor authentication on and its session
on that device has not passed it within `TOKULY_ACCOUNT_SWITCH_MFA_HOURS`.
`SignOut { all_accounts_on_device }` signs every account of the device out (a shared computer).
Web clients get the parked refresh tokens in a second HttpOnly cookie,
`tokuly_ra` (same path as `tokuly_rt`). Nothing links the accounts on the
server. Security log entries: `ACCOUNT_SWITCHED` (on the target account),
`SIGN_IN` with `added_account`.

## Opening the web from an app (hand-off)

See the module documentation of `handoff.rs`. `CreateWebHandoff` (a
session's own, never an API token's or a hand-off web session's; 60 an
account per 10 minutes) makes a single-use code, bound to the SHA-256 of a
verifier the app made (`verifier_hash`, `INVALID_VERIFIER` otherwise; PKCE's
idea), for a checked path of the web app (`INVALID_PATH` otherwise: a scheme, a host, `//`, a backslash, an
encoded one, an unknown first segment). The code lives 60 seconds in Valkey
under its keyed hash (`id:handoff:<hash>`). `GET <api prefix>/auth/handoff?code=…`
(60 an address per minute) does nothing without the verifier in the
`x-tokuly-handoff-verifier` header (a link someone was sent: login CSRF), takes
the code once, signs in only when the verifier matches, and makes a web session of the same
account with `sessions.linked_to` set to the app's session, sets `tokuly_rt`
(HttpOnly, SameSite=Lax, Secure on https) and answers `303` to the path;
any failure answers `303 /login` without a cookie. An app session holds one
such web session (a newer hand-off revokes the one before). It ends with the
app's session (trigger `sessions_revoke_linked`, and the token cache through
`forget_session_tokens`) and refreshes only while that session is live and
not parked; `revoke_all_sessions(keep)` keeps both halves of a pair.
Security log entry: `WEB_SIGN_IN_FROM_APP` (`session_id`, `app_session_id`).

## Configuration

| Variable | Default | |
|---|---|---|
| `TOKULY_REGISTRATION_OPEN` | `true` | |
| `TOKULY_MIN_AGE` | `13` | |
| `TOKULY_SESSION_DAYS` | `60` | Also the lifetime of the parked-accounts cookie. |
| `TOKULY_DELETION_GRACE_DAYS` | `30` | |
| `TOKULY_SPOTIFY_CLIENT_ID` / `_SECRET` | empty | Now playing and pinned songs; off when empty. |
| `TOKULY_SPOTIFY_API_URL` | `https://api.spotify.com` | A test double in tests. |
| `TOKULY_DEVICE_ACCOUNT_LIMIT` | `5` | Accounts per device for switching. |
| `TOKULY_ACCOUNT_SWITCH_MFA_HOURS` | `12` | |
| `TOKULY_PROFILE_LINKS_ALLOW_LOOPBACK` | `false` | Development and tests only; refused in production. |

## Limits

`UpdateProfileExtras` 60 an hour, `PinSong` 30 an hour, `RecheckProfileLinks`
5 an hour per account; `SuggestProfileTags` 120 a minute per account or
address; `ListDeviceAccounts` / `RemoveDeviceAccount` 120 a minute and
`SwitchAccount` 30 per 5 minutes per address, 10 per 5 minutes per target
session, plus the TOTP attempt limits of the target account.
