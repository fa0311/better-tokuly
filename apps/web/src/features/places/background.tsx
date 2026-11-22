import { useEffect } from "react";
import { useSessionStore } from "#/stores/session";
import { forgetDismissed } from "./dismissed";

/**
 * Runs while someone is signed in. Its one job: when the account signs
 * out, what this device remembered about its suggestions goes too.
 */
export default function PlacesBackground() {
  useEffect(
    () =>
      useSessionStore.subscribe((session) => {
        if (session.state.status === "signedOut") forgetDismissed();
      }),
    [],
  );
  return null;
}
