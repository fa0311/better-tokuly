import { QueryClient } from "@tanstack/react-query";
import { retryDelay, shouldRetry } from "@tokuly/core/query";

export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: 30_000,
        gcTime: 5 * 60_000,
        refetchOnWindowFocus: false,
        retry: shouldRetry,
        retryDelay,
        // Said out loud: TanStack Query makes a query with a `persister` (the realtime journal)
        // "offlineFirst" otherwise, and it would fail offline instead of waiting like the rest.
        networkMode: "online",
      },
      mutations: { retry: false },
    },
  });
}
