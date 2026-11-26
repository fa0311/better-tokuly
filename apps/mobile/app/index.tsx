import { Redirect } from "expo-router";
import { routeFor, usePendingLink } from "#/features/session/routing";
import { useSessionStore } from "#/features/session/store";

/** `/` (a plain start, `tokuly:///`): the map when signed in, sign-in otherwise. */
export default function Start() {
  const status = useSessionStore((s) => s.state.status);
  const link = usePendingLink((s) => s.link);
  const to = routeFor(status, "root", link);
  return to ? <Redirect href={to} /> : null;
}
