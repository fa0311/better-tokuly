import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { create } from "zustand";
import { FailsAlone } from "#/components/fails-alone";
import backgrounds from "#/generated/seams/background";

/**
 * Background features: what runs on every screen while someone is signed in
 * (realtime listeners that keep the cache current, sharing, uploads going on).
 *
 * A feature default-exports a component, usually rendering nothing, from
 * `src/features/<area>/background.tsx`:
 *
 *   export default function ChatBackground() {
 *     useChatSync();
 *     return null;
 *   }
 *
 * It is mounted inside the account's scope: a switch or a sign-out unmounts
 * it and mounts a new one for the next account, with an empty query cache.
 *
 * Its realtime listeners start at once (an event missed now is missed for
 * good). A first load that can wait — counts, a catalogue, a list loaded
 * ahead of its screen — waits for `useAfterFirstScreen()`, and only one that
 * reads: what sends for the account (sharing, push, uploads) starts at once,
 * so it never starts while the account is leaving.
 *
 * One that throws while rendering stops alone (`FailsAlone`): the screens
 * and the other background features go on.
 */
export function BackgroundFeatures() {
  useFirstScreenGate();
  return backgrounds.map((Background, index) => (
    <FailsAlone key={index}>
      <Background />
    </FailsAlone>
  ));
}

/**
 * Right after sign-in the map draws and the tab bar must take the first tap;
 * the first loads of every background feature would all land in those same
 * seconds and hold up the JS thread. They wait for the account's first loads
 * to be in and the JS thread to be idle, at most this long.
 */
const LATEST_MS = 5000;

const useFirstScreen = create<{ up: boolean }>(() => ({ up: false }));

/** Whether the signed-in account's first screen is up, and work that can wait may start. */
export function useAfterFirstScreen(): boolean {
  return useFirstScreen((state) => state.up);
}

function useFirstScreenGate(): void {
  const queryClient = useQueryClient();
  useEffect(() => {
    let idle: number | null = null;
    const open = () => {
      stop();
      useFirstScreen.setState({ up: true });
    };
    const openIfQuiet = () => {
      idle = null;
      if (queryClient.isFetching() === 0) open();
    };
    const whenQuiet = () => {
      if (idle === null && queryClient.isFetching() === 0) idle = requestIdleCallback(openIfQuiet);
    };
    const unsubscribe = queryClient.getQueryCache().subscribe(whenQuiet);
    const frame = requestAnimationFrame(whenQuiet);
    const latest = setTimeout(open, LATEST_MS);
    function stop() {
      unsubscribe();
      cancelAnimationFrame(frame);
      clearTimeout(latest);
      if (idle !== null) cancelIdleCallback(idle);
    }
    return () => {
      stop();
      useFirstScreen.setState({ up: false });
    };
  }, [queryClient]);
}
