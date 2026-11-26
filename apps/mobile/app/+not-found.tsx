import { Redirect, useLocalSearchParams, usePathname } from "expo-router";
import { screenFor } from "#/seams/links";

/**
 * A link the app has no route for: the screen a feature claims for that
 * path of the server (`#/seams/links`; a page only the web has opens in the
 * web view, signed in), otherwise the start of the app instead of a dead end.
 * The link's query goes along (`?t=` of a video, `?comment=`).
 */
export default function NotFound() {
  const pathname = usePathname();
  const { "not-found": _segments, ...params } = useLocalSearchParams();
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(params))
    for (const one of Array.isArray(value) ? value : [value])
      if (one !== undefined) query.append(key, one);
  const search = query.toString();
  const link = search ? `${pathname}?${search}` : pathname;
  return <Redirect href={(screenFor(link) ?? "/") as never} />;
}
