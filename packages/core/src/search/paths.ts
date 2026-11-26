import { search } from "@tokuly/api";
import { extraText, hitExtra } from "./kinds";

const K = search.SearchKind;

/**
 * Whether a path a domain published can be used as a link inside the app:
 * it starts with one "/" (not "//" or "/\\", which browsers read as another
 * site) and holds no control characters.
 */
export function isAppPath(path: string): boolean {
  // biome-ignore lint/suspicious/noControlCharactersInRegex: control characters are exactly what is refused.
  return /^\/(?![/\\])[^\u0000-\u001f]*$/.test(path);
}

/** A community's page. `/c/<handle>`, where its domain publishes it, is a video channel in Tokuly. */
function communityPath(handle: string): string {
  return `/g/${encodeURIComponent(handle)}`;
}

/**
 * Where a result leads, as a path of the web app (the phone opens it as it
 * opens any server link). People, channels, notes, hashtags, news, emoji,
 * communities and their threads have pages whose address the web app
 * decides; everything else (answers and audio rooms among them) opens at the
 * path its domain published. `undefined` when there is no usable path: the
 * app then searches for the hit instead.
 */
export function hitPath(
  hit: Pick<search.SearchHit, "kind" | "id" | "path" | "title" | "extraJson" | "owner">,
): string | undefined {
  const extra = hitExtra(hit);
  // A published path is only followed when it is a path of this site.
  const published = isAppPath(hit.path) ? hit.path : undefined;
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
    case K.EMOJI: {
      const name = extraText(extra, "name") || hit.title.replace(/^:|:$/g, "");
      return name ? `/emoji/${segment(name)}` : published;
    }
    case K.COMMUNITY: {
      const handle = extraText(extra, "handle");
      return handle ? communityPath(handle) : published;
    }
    case K.THREAD: {
      // Published as `/c/<community>/t/<thread>`: the same thread under the address for communities.
      const handle = extraText(extra, "community_handle");
      const thread = /^\/c\/[^/?#]+\/t\/([^/?#]+)$/.exec(hit.path)?.[1];
      return handle && thread ? `${communityPath(handle)}/t/${thread}` : published;
    }
    default:
      return published;
  }
}
