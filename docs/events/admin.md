# admin — events, jobs and configuration

Crate `tokuly-admin`, migration `1600_admin.sql`, API `tokuly.admin.v1`
(`ModerationService`, `UserAdminService`, `FeatureFlagService`,
`AnnouncementService`, `SystemService`, `ServerSettingsService`). No realtime events.

Every RPC needs the `moderator` or `admin` role (admins hold every role),
except `AnnouncementService.ListAnnouncements` / `GetAnnouncement`, which are
public. Each RPC's proto comment names its role.

## How a report becomes an action

```
engagement ── engagement.report.created ──▶ consumer "admin"
                                               │  one open case per subject (moderation_cases),
                                               │  one row per report (moderation_reports, idempotent on report_id)
                                               ▼
                         moderator: ModerationService.ResolveCase
          ┌───────────────┬──────────────────┬──────────────────┬─────────────────┐
       DISMISS      REMOVE_CONTENT          WARN            SUSPEND / BAN
          │      admin.content.removed   system.warning   set_banned_on
          │      system.content_removed  (to the owner)   + user_suspensions row
          │      (to the owner)                            + email to the owner
          └───────────────┴──────────────────┴──────────────────┴─────────────────┘
             system.report_resolved to every reporter · moderation_actions · admin_audit_log
```

WARN, SUSPEND and BAN can take the content down in the same resolution
(`remove_content`). `ModerationService.TakedownContent` removes content by
reference without a report; if an open case exists for the subject it is
resolved through the same path.

**Content reported without an owner.** A post in an anonymous channel
resolves with nobody as its owner, so `engagement.report.created` names
none. When a report arrives that way, and when a moderator takes such content
down by reference, the owner is asked from `HiddenOwners`, a lookup the
server hands in (`Admin::set_hidden_owners`; today the community domain).
The answer is stored as the case's owner and marked
(`moderation_cases.owner_hidden`), which lets a case warn, suspend or ban the
author and tells the author about a takedown. Without a lookup such cases
stay ownerless and cannot sanction anyone.

A hidden owner is handled the way the community handles it:

| | |
|---|---|
| Lists (`ListCases`) and the answers to changes (`AssignCase`, `ResolveCase`, `ReopenCase`, `TakedownContent`) | never name a hidden owner: `owner_withheld`, no `subject_owner`, and `target_withheld` without `target_user` on the actions. `ListCases` by `owner_id` leaves these cases out. |
| `GetCase` by a moderator who is **not involved** | names the owner and returns their moderation record, and writes `case.author_revealed` (actor, case, owner, time) to the audit log, every time. |
| `GetCase` by a moderator who **is involved** | `owner_withheld` and `conflict_of_interest`: another moderator has to handle the case. |
| `ResolveCase`, `TakedownContent` by a moderator who is involved | `CONFLICT_OF_INTEREST`, whatever the action. |

Involved means: the moderator reported the subject to site moderation (any
case about it), or the lookup says so (`HiddenOwners::is_party`; in the
community: wrote in that thread, is mentioned or answered by its author
there, reported it to the community's staff, or wrote the post). A case
marked hidden on a server without a lookup is withheld from everyone.

`GetCase` is the one place a hidden author is named. Everywhere else an
entry about an anonymous post is shown with `target_withheld`: without the
account and without anything that could be matched against the account's own
record.

| Where | What is withheld |
|---|---|
| moderation log (`ListModerationActions`, actions in `GetCase` unless that view named the author, answers to changes) | the account, `expires_at`, `reason`, `note`; `created_at` cut to the day (UTC). Left out entirely from the history of one account (by `user_id`, `GetUser`). A dismissal records no target. |
| a case shown without its author | `resolution_reason`; `resolved_at` cut to the day |
| moderation record (`UserModerationSummary`) | the account's anonymous posts, except in `GetCase` the one whose author it names |
| a suspension imposed for such a post | `case_id` (like a direct one; the reason stays) |
| audit log | the account, the reason, `suspension_id`, `replaced_suspension_id`, `expires_at` and `note` in snapshots; time cut to the day. Left out when filtered by that account. |

What stays visible: that an account is suspended, and since when (on a quiet
day that can be compared with the day such a case was resolved).

**Unmasking is limited.** `GetCase` names at most
`TOKULY_MODERATOR_REVEALS_PER_HOUR` hidden authors per staff member and hour
(admins included; a case opened again within the hour counts once); past
that, such a case answers `RATE_LIMITED`.

## Cases about staff

A case about a moderator's own content or about their account is not
theirs: `GetCase` gives them the case with `conflict_of_interest` and the
actions taken, but no reports (who reported them, what they wrote), notes or
owner record; `ListCaseReports`, `AssignCase`, `AddCaseNote`, `ReopenCase`
and `ResolveCase` answer `CONFLICT_OF_INTEREST`. Moderators cannot sanction
staff or change a staff account's verified badge (their own included):
admins do (`TARGET_IS_STAFF`).

## Evidence

An unsend, an edit or a deletion by the owner does not take the evidence
with it: every case holds its subject's uploads under the media holder
`admin:case:<case id>` from the moment a report files into it until
`TOKULY_MODERATION_EVIDENCE_DAYS` after it is resolved (a reopened case
keeps them). The server hands in the lookup (`Admin::set_report_evidence`,
trait `tokuly_admin::ReportEvidence`); it holds the uploads of chat
messages, the content whose sender can take its uploads back. What was
unsent before the report was filed cannot be held. A takedown still deletes
the uploads.

Everything a resolution does (case update, events, notifications, both
logs, a suspension's account lock) commits in one transaction. The account
is signed out right after it; identity's consumer of `identity.user.banned`
does so too, should that fail.

## Events emitted

| Topic | Payload | When |
|---|---|---|
| `admin.content.removed` | `{ subject_type, subject_id, reason, moderator_id }` (`tokuly_admin::events::ContentRemoved`) | A moderator took content down (case resolution or direct takedown). `subject_type` is `tokuly.common.v1.SubjectType` as its number. Actor: the moderator. |
| `admin.report.resolved` | `{ report_ids, action_taken }` (`tokuly_admin::events::ReportResolved`) | A moderator resolved a case. `report_ids` are the `report_id`s of every `engagement.report.created` filed into it; `action_taken` is false for a dismissal. Actor: the moderator. The engagement domain uses it to show reporters the outcome. |
| `admin.server_setting.changed` | `{ section, key, secret, live }` (`tokuly_admin::events::ServerSettingChanged`) | An admin saved or removed a server setting in 「サーバー設定」. Never carries the value. Actor: the admin. Nobody consumes it today; the processes reload the settings from the `sys.settings.changed` broadcast instead. |

**What the owning domain does with it:** delete or hide the subject (and
whatever hangs off it), unindex it, and treat an unknown or already removed
`subject_id` as done — the event can arrive twice, and a direct takedown is
not verified against the owning domain when the content is not visible to the
moderator. The admin domain already notifies the owner; do not notify again.
`SUBJECT_TYPE_USER` is never sent: accounts are suspended, not removed.

Account locks are announced by identity, not by this crate:
`identity.user.banned` / `identity.user.unbanned` (`{ user_id }`) follow every
suspension, ban, lift and expiry. Role and badge changes surface as
`identity.user.updated` where identity emits it.

## Events consumed

Durable consumer `admin` (`tokuly_admin::Admin::handle_event`).

| Topic | Payload | Effect |
|---|---|---|
| `engagement.report.created` | `{ report_id, reporter_id, subject_type, subject_id, subject_owner_id, category, text, subject_title, subject_path, reported_content }` (`tokuly_admin::events::ReportCreated`) | Files the report into the subject's open case, opening one if needed. `reported_content` (every version of content edited or withdrawn since; `subject_title` is only what the reporter could still see) is kept on the case from the latest report that carried it, up to 4000 characters, and cleared with the title when the owner's account is erased. It is shown like the reports: in `GetCase` only, and not in a case about the caller. Idempotent on `report_id`. `category` is one of `SPAM`, `HARASSMENT`, `HATE`, `SEXUAL`, `VIOLENCE`, `SELF_HARM`, `ILLEGAL`, `IMPERSONATION`, `COPYRIGHT`, `OTHER`; anything else is stored as `OTHER`. Reporter or owner IDs that match no live account are stored as unknown. |
| `identity.user.created`, `identity.user.email_verified` | not read | Grants `admin` to the account whose verified address is `TOKULY_ADMIN_USER`, if it now exists and the bootstrap has not happened yet. |
| `identity.user.deleted` | `{ user_id }` | Reports they filed lose reporter and text; cases about them or their content lose owner, title and path, and open ones are closed ("account deleted"); the moderation log keeps its rows with the user reference nulled (as target or moderator); the audit log nulls them as the affected account but keeps them as the actor (see "Privacy"); their suspension rows are deleted; they are removed from feature-flag allow-lists. |

## Notification kinds

All through `tokuly_core::outbound::notify`, no actor, Japanese and English.

| Kind | To | Channels | When |
|---|---|---|---|
| `system.warning` | the warned account | inbox + push | `ResolveCase` with WARN, `UserAdminService.WarnUser` |
| `system.content_removed` | the content owner | inbox + push | content taken down; `subject` is set, `path` is `/notifications` (the content is gone) |
| `system.report_resolved` | each distinct reporter of the case | inbox only | a case was resolved; `data = { case_id, action_taken }`, collapse key `report-resolved:{case_id}` |
| `announcement.published` | every active account | inbox + push | an announcement published with `notify_users`; `path` is `/news/{id}`, `data = { announcement_id }`, collapse key `announcement:{id}` |

`announcement.published` lands in notify's `feed` category (user-switchable,
normal urgency) rather than `system`.

"Active" for a broadcast: not deleted, not suspended, and holding a session
that is neither revoked nor expired and was active within
`TOKULY_ANNOUNCEMENT_NOTIFY_ACTIVE_DAYS`.

## Email

Suspended accounts are signed out and cannot read notifications, so these go
by mail (`core.mail`, in the account's locale; sent after commit, best effort):

- "アカウントが停止されました" / "Your Tokuly account has been suspended" — reason
  and end of the suspension in the account's time zone, or "無期限".
- "アカウントの停止が解除されました" / "Your Tokuly account is active again" — on a
  manual lift and on automatic expiry.

## Background work (`Admin::spawn_workers`, worker processes)

| What | Name | Notes |
|---|---|---|
| Event consumer | durable `admin` | topics above |
| Announcement broadcast | job `admin.announcement.broadcast` (`tokuly_admin::announcements::BroadcastJob { announcement_id }`) | Notifies active users in chunks of `TOKULY_ANNOUNCEMENT_NOTIFY_CHUNK`, one transaction per chunk; the cursor is stored on the announcement in that transaction, so retries and duplicate jobs resume instead of repeating. One at a time per process, lease 120 s, 5 attempts. A broadcast still unfinished an hour after it was queued is queued again. |
| Maintenance timer | every 60 s (first pass at start) | Lifts suspensions whose `expires_at` has passed (`Admin::expire_due_suspensions`); lets go of the evidence of cases resolved more than `TOKULY_MODERATION_EVIDENCE_DAYS` ago (`Admin::release_due_evidence`); queues the broadcast of announcements that became visible (`Admin::queue_due_broadcasts`) — this is what fires scheduled announcements. Safe with several workers. |

`PublishAnnouncement` also queues the broadcast directly when the
announcement is visible at once, so API-only processes need no timer.

## Privacy

The audit log is the record of what staff did with their powers. When a
staff account is erased, its entries keep their actor (shown as the erased
placeholder) and their snapshots; only references to the erased person as
the affected account are cut.

## Audit log

Every state change made through this API writes one `admin_audit_log` row in
the same transaction (identity-owned changes — roles, badge, sessions — right
after identity committed). Nothing in the API changes or deletes a row; only
the erasure of an account nulls its references. Role changes need the caller
to still be an admin at that moment, so two admins demoting each other leave
one of them. `SystemService.ListAuditLog` filters by an exact
action or by a prefix ending in a dot (`user.`).

| Action | Target | before / after |
|---|---|---|
| `case.assigned`, `case.unassigned` | case | assignee |
| `case.note_added` | case | note id |
| `case.resolved` | case | status, report count / action, content removed, note |
| `case.reopened` | case | the previous resolution |
| `content.removed` | subject (`<type number>:<id>`) | title, case, note |
| `user.warned` | user | note, case |
| `user.suspended` | user | was suspended, replaced suspension / new suspension, permanent, expiry, note, case |
| `user.unsuspended` | user | lifted suspension |
| `user.suspension_expired` | user (no actor) | lifted suspension |
| `user.verified_changed` | user | verified |
| `user.roles_changed` | user (no actor when from the `TOKULY_ADMIN_USER` bootstrap) | roles |
| `admin.bootstrapped` | `bootstrap` / the configured address (no actor; target user: who was promoted) | — written in one transaction with the grant; while it exists the bootstrap never runs again |
| `user.sessions_revoked` | user | number revoked |
| `flag.created`, `flag.updated`, `flag.deleted` | flag (its name) | full flag snapshot |
| `announcement.created`, `.updated`, `.published`, `.unpublished`, `.deleted` | announcement | titles, category, pinned, publish time, notify |
| `server_setting.updated` | setting (`section.key`) | `{ "value": … }`, `null` for no console value; secrets read `••••` |

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_ADMIN_USER` | *(empty)* | E-mail address of the account that runs the deployment. The account holding it gets the `admin` role once the address is verified (at startup, or the moment it is; the role shows up in its next access token, at the latest 15 minutes later), and only once per deployment and address: a later holder of the address, or a demoted bootstrap admin, is not promoted. A handle promotes nobody (whoever signed up first would hold it). |
| `TOKULY_ADMIN_STATS_TIME_ZONE` | `Asia/Tokyo` | Time zone whose midnight starts "today" in `GetServerStats` (new accounts today, DAU). |
| `TOKULY_ANNOUNCEMENT_NOTIFY_ACTIVE_DAYS` | `30` | A broadcast reaches accounts with a signed-in device active within this many days. |
| `TOKULY_ANNOUNCEMENT_NOTIFY_CHUNK` | `500` | Users notified per transaction during a broadcast (1–5000). |
| `TOKULY_MODERATOR_SANCTIONS_PER_HOUR` | `120` | Warnings, suspensions, bans and takedowns one moderator may issue per hour (admins are not limited). |
| `TOKULY_MODERATOR_REVEALS_PER_HOUR` | `30` | Hidden authors of anonymous posts one staff member (admins included) may be shown per hour, counted once per case. |
| `TOKULY_MODERATION_EVIDENCE_DAYS` | `90` | Days the uploads of reported content stay held for a case after it is resolved. |

Other rate limits (fixed): the public announcement RPCs allow 240 requests a
minute per client address; `GetServerStats` 30 a minute per admin;
`UpdateServerSetting` 60 a minute per admin.

## Server settings (「サーバー設定」)

`ServerSettingsService` is the console layer of `tokuly_core::settings`
(see `docs/backend/settings.md`): every setting of the binary with its
value, default, description and source. Precedence, highest first: the
environment / command line, `tokuly.toml`, this console, the default.
Secrets are stored sealed and shown as `••••` plus their last four
characters. Every change is recorded (`server_setting_changes`, the audit
entry and the event above) in one transaction. Live settings then apply in
every process at once; the rest show `restart_pending` until the listed
processes restart.

| Refused with | When |
|---|---|
| `SETTING_FILE_ONLY` | the setting's domain does not let the console change it |
| `SETTING_LOCKED` | the environment or the file sets it |
| `SETTING_MANAGED_ELSEWHERE` | it has its own screen (the chat AI providers at `/admin/chat-ai`) |
| `SETTING_INVALID` | wrong type, or the section's checks fail with all of its console values together |

Removing a value (`reset`) is always allowed. Console values a section does
not pass its checks with are set aside, the whole section together, at
start-up and on every reload; the page shows why (`console_rejected`). A
server that does not start with the console values starts without them.

## Getting back in

The bootstrap happens once per address. To make an account admin later
(lost access, a second operator):

```sh
docker compose exec api tokuly promote-admin you@example.com   # or a handle
```

It writes a `user.roles_changed` audit entry (no actor) and fails when there
is no such account or it already is an admin. To let `TOKULY_ADMIN_USER`
bootstrap again instead, delete its record:

```sql
DELETE FROM admin_audit_log WHERE action = 'admin.bootstrapped' AND target_type = 'bootstrap' AND target_id = 'you@example.com';
```
```

## For the server crate

```rust
let admin = Admin::new(core.clone(), identity.clone(), cfg.admin)?;   // also registers the "moderation" data exporter
let router = admin.register(router);
admin.spawn_workers();                                                 // worker processes
admin.grant_bootstrap_admin().await?;                                  // at startup: `TOKULY_ADMIN_USER`, if that verified address exists already
admin.set_hidden_owners(Arc<dyn tokuly_admin::HiddenOwners>);          // bridge: owners of content that is reported without one, and who is involved in it
admin.set_report_evidence(Arc<dyn tokuly_admin::ReportEvidence>);      // bridge: hold / release the uploads of reported content (holder admin:case:<id>)
```

`grant_bootstrap_admin` returns whether it granted the role;
`ensure_admin(handle_or_email)` promotes a named account unconditionally
(for tools and tests).

## Notes for other domains

- **Server stats** count rows of the content tables named in
  `stats.rs::CONTENT_TABLES`, skipping tables that do not exist. A domain whose
  main table is not listed there (or is named differently) only needs a line
  added.
- **Stamp pack review and payout review** are not in this crate: admin may only
  depend on identity, so those reviews are RPCs of the stamp and wallet domains,
  gated with `ctx.require_role(...)`. Taking a published pack down is covered
  here (`admin.content.removed` with `SUBJECT_TYPE_STAMP_PACK`).
- **Takedown by short ID** works for subject types whose domain registered a
  `SubjectResolver`; otherwise the UUID is required. The resolver is asked
  with the moderator as viewer, so content the moderator cannot see must be
  referenced by UUID as well; reported content referenced so is found the way
  its reporters see it (below), with its current title.
- **Reported content its owner hid** (a video made private, a comment hidden
  after the report): `GetCase` shows it as `ModerationCase.current_subject`
  (title and path, never an owner), resolved the way its reporters can still
  see it (`Admin::set_reported_subjects`). Not shown in a case about the
  caller, to a moderator involved in a hidden author's post, or to the
  subject's owner or reporters.
