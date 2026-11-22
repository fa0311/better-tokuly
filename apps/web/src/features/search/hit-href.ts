import { search } from "@tokuly/api";
import { extraText, hitExtra, isKind } from "./kinds";
import { searchHref } from "./params";

const K = search.SearchKind;

/**
 * Whether a path a domain published can be used as a link inside this app:
 * it starts with one "/" (not "//" or "/\\", which browsers read as another
 * site) and holds no control characters.
 */
export function isAppPath(path: string): boolean {
  // biome-ignore lint/suspicious/noControlCharactersInRegex: control characters are exactly what is refused.
  return /^\/(?![/\\])[^\u0000-\u001f]*$/.test(path);
}

/**
 * Where a result leads in this app. People, channels, notes, hashtags and
 * news have pages whose address the web app decides; everything else opens at
 * the path its domain published.
 */
export function hitHref(hit: Pick<search.SearchHit, "kind" | "id" | "path" | "title" | "extraJson" | "owner">): string {
  const extra = hitExtra(hit);
  // A published path is only followed when it is a path of this site; otherwise the hit is searched for.
  const published = isAppPath(hit.path) ? hit.path : searchHref(hit.title || hit.id, { kind: isKind(hit.kind) ? hit.kind : undefined });
  const segment = encodeURIComponent;
  switch (hit.kind) {
    case K.USER: {
      const handle = extraText(extra, "handle") || hit.owner?.handle;
      return handle ? `/u/${segment(handle)}` : published;
    }
    case K.CHANNEL: {
      const handle = extraText(extra, "handle");
      return handle ? `/c/${segment(handle)}` : published;
    }
    case K.NOTE:
      return `/notes/${segment(hit.id)}`;
    case K.HASHTAG: {
      const tag = extraText(extra, "tag") || hit.title.replace(/^[#＃]+/, "");
      return tag ? `/tag/${segment(tag)}` : published;
    }
    case K.NEWS:
      return `/news/${segment(hit.id)}`;
    default:
      return published;
  }
}
