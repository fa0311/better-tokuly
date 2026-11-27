import { QueryClient, type QueryClientConfig } from "@tanstack/react-query";

/**
 * A query client for a test. Its cache is never garbage-collected: a collection is a timer (five
 * minutes by default) that outlives the test and keeps jest from exiting.
 */
export function testQueryClient({
  defaultOptions,
  ...config
}: QueryClientConfig = {}): QueryClient {
  return new QueryClient({
    ...config,
    defaultOptions: {
      ...defaultOptions,
      queries: { ...defaultOptions?.queries, gcTime: Number.POSITIVE_INFINITY },
      mutations: { ...defaultOptions?.mutations, gcTime: Number.POSITIVE_INFINITY },
    },
  });
}
