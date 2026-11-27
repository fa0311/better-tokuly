# identity — events, jobs and configuration

Crate `tokuly-identity`, migration `0100_identity.sql`, API `tokuly.identity.v1`
(`AuthService`, `AccountService`, `ProfileService`). Depends on `core`,
`proto` and `media`. Every other domain depends on it.

## Events emitted

All are written on the transaction of the change (the transactional outbox),
except `identity.export.ready`.

| Topic | Payload | When |
|---|---|---|
| `identity.user.created` | `{ user_id, handle, invite_code, minor }` | A person signed up. Never for bots. `minor`: under 18 that day; start the account with your defaults for minors. |
| `identity.user.birthdate_given` | `{ user_id, minor }` (`maintenance::BirthdateGiven`) | An account without a birthdate gave one (`SetBirthdate`), or (once, migration `0101`) an account under 18 gave it before the defaults for minors came. Apply what you would have at sign-up, by `minor`, unless the person chose otherwise. Emitted with `identity.user.privacy_changed`: the settings that apply changed for either age group. |
| `identity.user.updated` | `{ user_id }` | The account's public face changed: profile fields, the verified badge, the プロフ帳. Search rebuilds the account's document on it. |
| `identity.profile.updated` | `{ user_id, changed: [group…] }` | The プロフ帳 changed. `changed` names the groups: `mbti`, `blood_type`, `pronouns`, `long_bio`, `theme`, `visibility`, `oshi`, `tags`, `links`, `qa_cards`, `pinned_song`. Carries no text: a consumer reads what it may show (audiences apply). Emitted with `identity.user.updated`. |
| `identity.user.status_changed` | `{ user_id }` | |
| `identity.user.privacy_changed` | `{ user_id }` | Privacy settings changed, including who sees the birthday (also when the plans domain's switch changed it). |
| `identity.user.deletion_requested` | `{ user_id }` | |
| `identity.user.email_verified` | `{ user_id }` | The account's address was confirmed (verification, change or undo of a change). |
| `identity.user.deleted` | `{ user_id }` | The account was erased: every domain removes that user's data. |
| `identity.user.banned` / `identity.user.unbanned` | `{ user_id }` | |
| `identity.user.recovery_codes_revoked` | `{ user_id }` | The account's two-step recovery codes stopped working (`rekey --forget-recovery-codes`, a leaked secret). |
| `identity.export.ready` | `{ user_id, export_id }` | |

**No activity entries for profile edits**: an activity entry goes to friends
as a whole, while each field has its own audience. Domains that want to
react follow `identity.profile.updated` and read the field under its
audience.

## Events consumed

Durable `identity-sessions`:

| Topic | Effect |
|---|---|
| `identity.user.banned` | Its own event: every session the account had when it was banned is revoked and its access tokens refused at once (`Identity::sign_out_banned`), parked sessions on other devices included. Whoever banned does this right after the commit too; the event makes it at least once. Sessions made after the ban are left alone. |
| `identity.user.deleted` | Its own event: the dead letters that mention the account are removed (`tokuly_core::dead_letters::erase_users`, `docs/events.md`). |

Durable `identity-mail`:

| Topic | Effect |
|---|---|
| `identity.user.recovery_codes_revoked` | Its own event: the account's address is told, with a link to the two-step settings (`Identity::tell_recovery_codes_revoked`). Once per event: a send is noted in Valkey; one cut short is sent again after five minutes. |

## Jobs and timers

| Name | What |
|---|---|
| `identity.export` | Builds the personal data archive (the account, the プロフ帳 with its audiences, settings, sessions with their switch marks, every domain's exporter) and mails a link. |
| `identity.profile.links` | `{ user_id }`: fetches the account's profile links that are due (never checked, or checked a week ago) through `tokuly_core::egress` (512 KiB, 5 s) and marks a link verified while the page has an `<a>` or `<link>` with `rel="me"` pointing at `<public url>/u/<handle>` or `/@<handle>`. At most 20 never-checked links per account and day. Queued after links or the handle change, and by `RecheckProfileLinks`. Idempotent. |
| timer, 6 h | Queues `identity.profile.links` for accounts with links that are due. |
| timer, 1 h | Erases accounts whose deletion grace period ran out (the プロフ帳 and its uploads included) and emits `identity.user.deleted`; removes dead letters about accounts erased in the last day that were buried after the event was handled. |

## Public Rust API for other domains

- `users::{summaries, summaries_ordered, summary, pick, filter_active, …}`, `settings::{privacy_of, privacy_of_many}`
  (the settings as they apply: a minor's within the limits), `settings::{findable_by_anyone, findable_by_friends_of_friends}`.
- **Minors** (`age`): two rules. Defaults are decided by the age on the day the birthdate is given and
  stored (the two events above say which group); limits follow the age today: `is_minor(core, id)`,
  `minors_among(core, ids)`, `minor_on(birthdate, today)`. No birthdate counts as a minor, a bot goes by
  its owner, `ADULT_AGE` is 18. Strangers put in touch by position or school meet within their
  `AgeGroup` only: `age_group(core, id)`, `age_groups_among(core, ids)` leave accounts without a
  birthdate out, `adult_if_born_by(today)` for a query. Under the minimum age an account is too young
  (`too_young_on`, `Me.too_young`, `Identity::too_young(birthdate, time_zone)`, by the date in the account's time zone, for what acts for an account without a session): its sessions reach `tokuly_core::auth::reachable_while_too_young` only, the tokens already out included (`Core::hold_too_young`).
- **Bots** (`bots`): `create_bot_account(tx, owner, handle, display_name, bio) -> Uuid`, `set_bot_owner(tx, bot, owner)`, `is_bot(core, id)`, `bots_among(core, ids) -> HashSet`, `bot_owners(core, ids) -> HashMap<bot, owner>`, and the one rule for places where only people count: `humans_only(core, ids) -> Vec<Uuid>` (or `NOT u.bot` in a join on `users`). Friend suggestions, leaderboards, rankings, nearby discovery, intro counts and anonymity group sizes use it.
- **Birthdays** (`birthdays`): `shared_birthdays(core, owners, closeness) -> HashMap<Uuid, SharedBirthday { month, day, year: Option }>` (batched, the caller says who is a friend), `Identity::visible_birthdays(viewer, owners)` (friendships and blocks from the profile extensions), `set_birthday_audience(core, user, audience, show_year)`, `audience_of(&PrivacySettings)`.
- **プロフ帳** (`extras`): `search_terms(core, ids, &locked) -> HashMap<Uuid, ProfileSearchTerms { text, tags }>` (public fields of unlocked accounts only, for search documents), `Extras::load(..).view(&Viewer)`, `reaches(visibility, circle, &Viewer)` (an allowlist: unknown values reach only the owner), `tag_key`.
- **Moderation**: `Identity::extras_for_moderation(user_id)` — the プロフ帳 as far as it reaches anybody, with its audiences (not "only me" fields or hidden cards). Not an RPC: the admin domain calls it from `ModerationService.GetReportedProfile`, for an open case about the account, and records `case.profile_viewed`.
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

The model is in the module documentation of `handoff.rs`. The contract:

- `CreateWebHandoff` (a session's own, never an API token's or a hand-off
  web session's; 60 an account per 10 minutes) makes a single-use code that
  lives 60 seconds, bound to the SHA-256 of a verifier the app made
  (`INVALID_VERIFIER`), for a checked path of the web app (`INVALID_PATH`).
- `GET <api prefix>/auth/handoff?code=…` (60 an address per minute) needs
  the verifier in the `x-tokuly-handoff-verifier` header, so a link someone
  was sent signs nobody in. It sets `tokuly_rt` and answers `303` to the
  path; any failure answers `303 /login` without a cookie.
- The web session is linked to the app's session: an app session holds one
  (a newer hand-off revokes the one before), it ends with the app's session
  and refreshes only while that session is live and not parked.
- Security log entry: `WEB_SIGN_IN_FROM_APP` (`session_id`, `app_session_id`).

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

`UpdateProfileExtras` 60 an hour, `PinSong` 30 an hour, `SetBirthdate` 10 an hour, `RecheckProfileLinks`
5 an hour per account; `SuggestProfileTags` 120 a minute per account or
address; `ListDeviceAccounts` / `RemoveDeviceAccount` 120 a minute and
`SwitchAccount` 30 per 5 minutes per address, 10 per 5 minutes per target
session, plus the TOTP attempt limits of the target account.
