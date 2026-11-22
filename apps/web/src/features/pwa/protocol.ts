import { useEffect } from "react";
import { realtime } from "#/lib/realtime";
import { checkForUpdate, useWorker } from "./service-worker";

/**
 * The realtime gateway says which version of its protocol it speaks. When
 * that is newer than the one this page was built for, the server was
 * updated underneath it and pushed events no longer reach it: the page is
 * behind, exactly as when a newer version of the app waits, so the same
 * "new version" toast offers the reload.
 */
export function useProtocolWatch(): void {
  useEffect(
    () =>
      realtime().onOutdated(() => {
        useWorker.setState({ outdated: true });
        // Fetch the new version now, so that the reload lands in it.
        void checkForUpdate();
      }),
    [],
  );
}
