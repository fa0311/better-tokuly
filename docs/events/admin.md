# admin — events, jobs and configuration

Crate `tokuly-admin`, migration `1600_admin.sql`, API `tokuly.admin.v1`
(`ModerationService`, `UserAdminService`, `FeatureFlagService`,
`AnnouncementService`, `SystemService`). No realtime events.

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
          │      admin.content.removed   system.warning   Identity::set_banned
          │      system.content_removed  (to the owner)   + user_suspensions row
          │      (to the owner)                            + email to the owner
          └───────────────┴──────────────────┴──────────────────┴─────────────────┘
             system.report_resolved to every reporter · moderation_actions · admin_audit_log
```

WARN, SUSPEND and BAN can take the content down in the same resolution
(`remove_content`). `ModerationService.TakedownContent` removes content by
reference without a report; if an open case exists for the subject it is
resolved through the same path.

Everything a resolution does (case update, events, notifications, both logs)
commits in one transaction. The account lock itself is committed by identity
(`set_banned`) just before; see `sanctions.rs` for why that order is safe.

## Events emitted

| Topic | Payload | When |
|---|---|---|
| `admin.content.removed` | `{ subject_type, subject_id, reason, moderator_id }` (`tokuly_admin::events::ContentRemoved`) | A moderator took content down (case resolution or direct takedown). `subject_type` is `tokuly.common.v1.SubjectType` as its number. Actor: the moderator. |
| `admin.report.resolved` | `{ report_ids, action_taken }` (`tokuly_admin::events::ReportResolved`) | A moderator resolved a case. `report_ids` are the `report_id`s of every `engagement.report.created` filed into it; `action_taken` is false for a dismissal. Actor: the moderator. The engagement domain uses it to show reporters the outcome. |

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
| `engagement.report.created` | `{ report_id, reporter_id, subject_type, subject_id, subject_owner_id, category, text, subject_title, subject_path }` (`tokuly_admin::events::ReportCreated`) | Files the report into the subject's open case, opening one if needed. Idempotent on `report_id`. `category` is one of `SPAM`, `HARASSMENT`, `HATE`, `SEXUAL`, `VIOLENCE`, `SELF_HARM`, `ILLEGAL`, `IMPERSONATION`, `COPYRIGHT`, `OTHER`; anything else is stored as `OTHER`. Reporter or owner IDs that match no live account are stored as unknown. |
| `identity.user.created` | not read | Grants `admin` to the `TOKULY_ADMIN_USER` account if it now exists. |
| `identity.user.deleted` | `{ user_id }` | Reports they filed lose reporter and text; cases about them or their content lose owner, title and path, and open ones are closed ("account deleted"); the moderation log and the audit log keep their rows with the user reference nulled (as target, moderator or actor); their suspension rows are deleted; they are removed from feature-flag allow-lists. |

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
| Maintenance timer | every 60 s (first pass at start) | Lifts suspensions whose `expires_at` has passed (`Admin::expire_due_suspensions`); queues the broadcast of announcements that became visible (`Admin::queue_due_broadcasts`) — this is what fires scheduled announcements. Safe with several workers. |

`PublishAnnouncement` also queues the broadcast directly when the
announcement is visible at once, so API-only processes need no timer.

## Audit log

Every state change made through this API writes one `admin_audit_log` row in
the same transaction (identity-owned changes — roles, badge, sessions — right
after identity committed). `SystemService.ListAuditLog` filters by an exact
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
| `user.sessions_revoked` | user | number revoked |
| `flag.created`, `flag.updated`, `flag.deleted` | flag (its name) | full flag snapshot |
| `announcement.created`, `.updated`, `.published`, `.unpublished`, `.deleted` | announcement | titles, category, pinned, publish time, notify |

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_ADMIN_USER` | *(empty)* | Handle or email address of the account that runs the deployment. It gets the `admin` role at startup or, when it does not exist yet, the moment it signs up (the role shows up in its next access token, at the latest 15 minutes later). |
| `TOKULY_ADMIN_STATS_TIME_ZONE` | `Asia/Tokyo` | Time zone whose midnight starts "today" in `GetServerStats` (new accounts today, DAU). |
| `TOKULY_ANNOUNCEMENT_NOTIFY_ACTIVE_DAYS` | `30` | A broadcast reaches accounts with a signed-in device active within this many days. |
| `TOKULY_ANNOUNCEMENT_NOTIFY_CHUNK` | `500` | Users notified per transaction during a broadcast (1–5000). |
| `TOKULY_MODERATOR_SANCTIONS_PER_HOUR` | `120` | Warnings, suspensions, bans and takedowns one moderator may issue per hour (admins are not limited). |

Other rate limits (fixed): the public announcement RPCs allow 240 requests a
minute per client address; `GetServerStats` 30 a minute per admin.

## For the server crate

```rust
let admin = Admin::new(core.clone(), identity.clone(), cfg.admin)?;   // also registers the "moderation" data exporter
let router = admin.register(router);
admin.spawn_workers();                                                 // worker processes
admin.grant_bootstrap_admin().await?;                                  // at startup: `TOKULY_ADMIN_USER`, if that account exists already
```

`grant_bootstrap_admin` (and `ensure_admin(handle_or_email)`, which it calls)
returns `true` when it granted the role, `false` when the account already was
an admin or does not exist yet. An account that does not exist yet is
promoted by the worker when it signs up.

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
  referenced by UUID as well.
