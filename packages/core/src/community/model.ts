import { type common, community } from "@tokuly/api";

/**
 * Pure rules of the community screens: what a viewer may do (always read
 * from the server's `viewer` blocks, never from a copy of the permission
 * table), the addresses of community pages in the apps, and small helpers for
 * forms.
 */

const P = community.Permission;
const S = community.MembershipStatus;

export function isMember(c: Pick<community.Community, "viewer"> | undefined): boolean {
  return c?.viewer?.status === S.MEMBER;
}

/** Whether the server granted the caller a permission in this community. */
export function can(
  c: Pick<community.Community, "viewer"> | undefined,
  permission: community.Permission,
): boolean {
  return c?.viewer?.permissions.includes(permission) ?? false;
}

/** The permissions that open one of the staff pages. */
export const STAFF_PERMISSIONS = [
  P.MODERATE_CONTENT,
  P.MANAGE_MEMBERS,
  P.VIEW_MODERATION_LOG,
  P.MANAGE_SAFETY,
  P.MANAGE_CHANNELS,
  P.MANAGE_COMMUNITY,
] as const;

export function isStaff(c: Pick<community.Community, "viewer"> | undefined): boolean {
  return STAFF_PERMISSIONS.some((permission) => can(c, permission));
}

/**
 * Whether the account's site-wide roles let it read every community (the
 * server's rule: `admin` and `moderator` handle reports anywhere).
 */
export function isSiteModerator(roles: readonly string[] | undefined): boolean {
  return roles?.some((role) => role === "admin" || role === "moderator") ?? false;
}

/**
 * Whether the community's boards, events and members can be read by the
 * caller: anyone in a public community, its members, and site moderation.
 */
export function canRead(c: community.Community | undefined, roles?: readonly string[]): boolean {
  if (!c) return false;
  return (
    c.visibility === community.CommunityVisibility.PUBLIC || isMember(c) || isSiteModerator(roles)
  );
}

/** The caller is timed out until this moment (epoch ms), or 0. */
export function timeoutUntilMs(
  viewer: Pick<community.CommunityViewer, "timeoutUntil"> | undefined,
  nowMs: number,
): number {
  const until = viewer?.timeoutUntil;
  if (!until) return 0;
  const ms = Number(until.seconds) * 1000;
  return ms > nowMs ? ms : 0;
}

// ───────────── Addresses ─────────────
//
// The server publishes community pages under `/c/<handle>/…`; in Tokuly
// `/c/<handle>` is a video channel, so communities live under `/g/<handle>`
// and invites under `/join/<code>`.

const segment = encodeURIComponent;

export const communityHref = (handle: string): string => `/g/${segment(handle)}`;
export const threadHref = (handle: string, threadId: string): string =>
  `${communityHref(handle)}/t/${segment(threadId)}`;
export const eventHref = (handle: string, eventId: string): string =>
  `${communityHref(handle)}/events/${segment(eventId)}`;
export const inviteHref = (code: string): string => `/join/${segment(code)}`;

/** The staff pages the community domain links to (`/c/<handle>/<page>`), by their section here. */
const STAFF_PAGES: Record<string, string> = {
  requests: "manage/requests",
  review: "manage/queue",
  log: "manage/log",
};

/**
 * A path the community domain published (`/c/<handle>`, `/c/<handle>/t/<id>`,
 * `/c/<handle>/events/<id>`, `/c/<handle>/requests|review|log`, `/c/join/<code>`)
 * as this app's address for the same page, query and fragment kept (`?reply=…`);
 * `undefined` for anything else.
 */
export function appPathOf(published: string): string | undefined {
  const text = published.trim();
  // A protocol-relative address names another site.
  if (text.startsWith("//")) return undefined;
  let url: URL;
  try {
    url = new URL(text, "http://app.invalid");
  } catch {
    return undefined;
  }
  const rest = url.search + url.hash;
  const path = url.pathname;
  const invite = /^\/c\/join\/([^/]+)$/.exec(path);
  if (invite) return `/join/${invite[1]}${rest}`;
  const page = /^\/c\/([^/]+)((?:\/t\/[^/]+)|(?:\/events\/[^/]+))?$/.exec(path);
  if (page) return `/g/${page[1]}${page[2] ?? ""}${rest}`;
  const staff = /^\/c\/([^/]+)\/([a-z]+)$/.exec(path);
  const section = staff?.[2] && STAFF_PAGES[staff[2]];
  return staff && section ? `/g/${staff[1]}/${section}${rest}` : undefined;
}

/** The code inside whatever someone pasted: a bare code, or an invite link of either shape. */
export function inviteCodeOf(input: string): string {
  const text = input.trim();
  const match = /\/join\/([A-Za-z0-9]+)(?:[/?#].*)?$/.exec(text);
  return match?.[1] ?? text;
}

// ───────────── Roles ─────────────

const R = community.MemberRole;

export const ROLE_KEYS: Record<community.MemberRole, string> = {
  [R.UNSPECIFIED]: "member",
  [R.MEMBER]: "member",
  [R.MODERATOR]: "moderator",
  [R.ADMIN]: "admin",
  [R.OWNER]: "owner",
};

/**
 * The roles the caller could give a member, as far as a client can know
 * before asking: admins appoint and dismiss moderators, the owner also
 * admins, and nobody acts on a member of their own role or above. The
 * server decides; this only keeps impossible entries out of a menu.
 */
export function assignableRoles(
  viewerRole: community.MemberRole,
  targetRole: community.MemberRole,
): community.MemberRole[] {
  if (viewerRole <= targetRole) return [];
  const all =
    viewerRole === R.OWNER
      ? [R.MEMBER, R.MODERATOR, R.ADMIN]
      : viewerRole === R.ADMIN
        ? [R.MEMBER, R.MODERATOR]
        : [];
  return all.filter((role) => role !== targetRole);
}

// ───────────── Posts ─────────────

/** The name a pseudonym is shown by: "あおいペンギン #k3f9". */
export function pseudonymName(pseudonym: community.Pseudonym, language: string): string {
  const name = (language.startsWith("ja") ? pseudonym.name?.ja : pseudonym.name?.en) || "";
  const shown = name || pseudonym.name?.ja || pseudonym.name?.en || "";
  return pseudonym.tag ? `${shown} #${pseudonym.tag}` : shown;
}

/** A `#rrggbb` colour, or nothing when the value is anything else (it goes into a style). */
export function safeColor(color: string): string | undefined {
  return /^#[0-9a-fA-F]{6}$/.test(color) ? color : undefined;
}

/** A hot list repeats threads across pages while scores move: keep the first of each. */
export function uniqueById<T extends { id: string }>(items: T[]): T[] {
  const seen = new Set<string>();
  return items.filter((item) => {
    if (seen.has(item.id)) return false;
    seen.add(item.id);
    return true;
  });
}

/** Share of the votes an option holds, 0-100. */
export function votePercent(option: community.PollOption, poll: community.Poll): number {
  const total = poll.options.reduce((sum, each) => sum + each.voteCount, 0);
  return total > 0 ? Math.round((option.voteCount / total) * 100) : 0;
}

/** Tags as typed ("#a, b　c") → normalized, unique, at most `max`. */
export function parseTags(input: string, max: number): string[] {
  const tags = input
    .split(/[\s,、，]+/)
    .map((tag) => tag.replace(/^[#＃]+/, "").toLowerCase())
    .filter(Boolean);
  return [...new Set(tags)].slice(0, max);
}

/** Splits a body into plain text and the `@handle`s the server recognised as members. */
export function splitMentions(
  body: string,
  mentions: Pick<community.Mention, "handle">[],
): { text: string; handle?: string }[] {
  if (mentions.length === 0) return [{ text: body }];
  const known = new Set(mentions.map((mention) => mention.handle.toLowerCase()));
  const parts: { text: string; handle?: string }[] = [];
  let last = 0;
  for (const match of body.matchAll(/@([A-Za-z0-9_]+)/g)) {
    const handle = match[1] ?? "";
    if (!known.has(handle.toLowerCase())) continue;
    const start = match.index ?? 0;
    if (start > last) parts.push({ text: body.slice(last, start) });
    parts.push({ text: match[0], handle });
    last = start + match[0].length;
  }
  if (last < body.length) parts.push({ text: body.slice(last) });
  return parts;
}

// ───────────── Events ─────────────

export type EventPhase = "upcoming" | "running" | "over";

export function eventPhase(
  event: Pick<community.CalendarEvent, "startsAt" | "endsAt">,
  nowMs: number,
): EventPhase {
  const start = Number(event.startsAt?.seconds ?? 0n) * 1000;
  const end = Number(event.endsAt?.seconds ?? 0n) * 1000;
  if (nowMs < start) return "upcoming";
  return nowMs <= end ? "running" : "over";
}

/** Check-in opens an hour before the start and closes six hours after the end. */
export function checkInOpen(
  event: Pick<community.CalendarEvent, "startsAt" | "endsAt" | "cancelled">,
  nowMs: number,
): boolean {
  if (event.cancelled) return false;
  const start = Number(event.startsAt?.seconds ?? 0n) * 1000;
  const end = Number(event.endsAt?.seconds ?? 0n) * 1000;
  return nowMs >= start - 3_600_000 && nowMs <= end + 6 * 3_600_000;
}

/** Places left, or `undefined` when the event has no limit. */
export function placesLeft(
  event: Pick<community.CalendarEvent, "capacity" | "goingCount">,
): number | undefined {
  return event.capacity > 0 ? Math.max(0, event.capacity - event.goingCount) : undefined;
}

/** Smallest stored variant of an image that covers `pixels` on a 2x screen. */
export function imageUrl(image: common.Image | undefined, pixels: number): string | undefined {
  if (!image) return undefined;
  const wanted = pixels * 2;
  const fitting = image.variants.filter((v) => v.size >= wanted).sort((a, b) => a.size - b.size)[0];
  return fitting?.url || image.url || undefined;
}

/** The community handle inside a path the domain published (`/c/<handle>/…`); empty when there is none. */
export function handleOfPath(published: string): string {
  const match = /^\/c\/([^/?#]+)/.exec(published.trim());
  return match?.[1] && match[1] !== "join" ? decodeURIComponent(match[1]) : "";
}
