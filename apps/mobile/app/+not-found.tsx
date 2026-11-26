import { Redirect, usePathname } from "expo-router";
import { screenFor } from "#/seams/links";

/**
 * A link the app has no route for: the screen a feature claims for that
 * path of the server (`#/seams/links`; a page only the web has opens in the
 * web view, signed in), otherwise the start of the app instead of a dead end.
 */
export default function NotFound() {
  const pathname = usePathname();
  return <Redirect href={(screenFor(pathname) ?? "/") as never} />;
}
