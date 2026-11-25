import { QueryClient } from "@tanstack/react-query";
import { shouldRetry } from "@tokuly/core/query";

/** One per signed-in account: switching accounts starts with an empty cache. */
export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { staleTime: 30_000, gcTime: 5 * 60_000, retry: shouldRetry },
      mutations: { retry: false },
    },
  });
}
