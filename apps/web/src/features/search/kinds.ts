import { search } from "@tokuly/api";

const K = search.SearchKind;

/** Every kind of thing search can find, in the order their tabs are shown. */
export const KINDS = [K.USER, K.CHANNEL, K.NOTE, K.VIDEO, K.SHORT, K.LIVE, K.CLIP, K.PLAYLIST, K.STAMP_PACK, K.PLACE, K.HASHTAG, K.NEWS] as const;
export type Kind = (typeof KINDS)[number];

/** A kind's name in the address (`/search?kind=people`) and in the `kinds.*` strings. */
const SLUGS: Record<Kind, string> = {
  [K.USER]: "people",
  [K.CHANNEL]: "channels",
  [K.NOTE]: "notes",
  [K.VIDEO]: "videos",
  [K.SHORT]: "shorts",
  [K.LIVE]: "live",
  [K.CLIP]: "clips",
  [K.PLAYLIST]: "playlists",
  [K.STAMP_PACK]: "stamps",
  [K.PLACE]: "places",
  [K.HASHTAG]: "tags",
  [K.NEWS]: "news",
};

export function isKind(value: search.SearchKind): value is Kind {
  return (KINDS as readonly search.SearchKind[]).includes(value);
}

export function kindSlug(kind: Kind): string {
  return SLUGS[kind];
}

export function kindFromSlug(slug: unknown): Kind | undefined {
  return KINDS.find((kind) => SLUGS[kind] === slug);
}

/** The kind-specific fields a domain published with its document (`SearchHit.extra_json`). */
export type HitExtra = Record<string, unknown>;

export function hitExtra(hit: Pick<search.SearchHit, "extraJson">): HitExtra {
  try {
    const parsed: unknown = JSON.parse(hit.extraJson || "{}");
    return parsed !== null && typeof parsed === "object" && !Array.isArray(parsed) ? (parsed as HitExtra) : {};
  } catch {
    return {};
  }
}

export const extraText = (extra: HitExtra, key: string): string => (typeof extra[key] === "string" ? (extra[key] as string) : "");
export const extraNumber = (extra: HitExtra, key: string): number | undefined => (typeof extra[key] === "number" && Number.isFinite(extra[key]) ? (extra[key] as number) : undefined);
export const extraFlag = (extra: HitExtra, key: string): boolean => extra[key] === true;

/**
 * Where a result leads in this app. People, channels, notes, hashtags and
 * news have pages whose address the web app decides; everything else opens at
 * the path its domain published.
 */
export function hitHref(hit: Pick<search.SearchHit, "kind" | "id" | "path" | "title" | "extraJson" | "owner">): string {
  const extra = hitExtra(hit);
  const segment = encodeURIComponent;
  switch (hit.kind) {
    case K.USER: {
      const handle = extraText(extra, "handle") || hit.owner?.handle;
      return handle ? `/u/${segment(handle)}` : hit.path;
    }
    case K.CHANNEL: {
      const handle = extraText(extra, "handle");
      return handle ? `/c/${segment(handle)}` : hit.path;
    }
    case K.NOTE:
      return `/notes/${segment(hit.id)}`;
    case K.HASHTAG: {
      const tag = extraText(extra, "tag") || hit.title.replace(/^[#＃]+/, "");
      return tag ? `/tag/${segment(tag)}` : hit.path;
    }
    case K.NEWS:
      return `/news/${segment(hit.id)}`;
    default:
      return hit.path;
  }
}
