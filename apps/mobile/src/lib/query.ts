import { focusManager, onlineManager, QueryClient } from "@tanstack/react-query";
import { retryDelay, shouldRetry } from "@tokuly/core/query";
import { addNetworkStateListener } from "expo-network";
import { AppState } from "react-native";

/** One per signed-in account: switching accounts starts with an empty cache. */
export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { staleTime: 30_000, gcTime: 5 * 60_000, retry: shouldRetry, retryDelay },
      mutations: { retry: false },
    },
  });
}

// What a browser tells TanStack Query by itself, the phone tells it here: back in front, what is
// on screen and stale is read again; offline, queries wait instead of failing, and go on when
// the network is back. A network of unknown state counts as online.
focusManager.setEventListener((setFocused) => {
  const subscription = AppState.addEventListener("change", (state) =>
    setFocused(state === "active"),
  );
  return () => subscription.remove();
});
onlineManager.setEventListener((setOnline) => {
  const subscription = addNetworkStateListener((state) => setOnline(state.isConnected !== false));
  return () => subscription.remove();
});
