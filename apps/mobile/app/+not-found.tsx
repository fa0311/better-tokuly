import { Redirect, useGlobalSearchParams, usePathname } from "expo-router";
import { routeOf, targetOf } from "#/features/notifications/target";

/**
 * A link the app has no screen for: a page of the web app (a note, a live
 * stream, a group) opens in the web view, signed in; anything else starts
 * the app as usual instead of a dead end.
 */
export default function NotFound() {
  const pathname = usePathname();
  const params = useGlobalSearchParams<Record<string, string>>();
  const search = new URLSearchParams(params).toString();
  const route = routeOf(targetOf(search ? `${pathname}?${search}` : pathname));
  return <Redirect href={(route ?? "/") as never} />;
}
