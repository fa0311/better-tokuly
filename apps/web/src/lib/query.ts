import { QueryClient } from "@tanstack/react-query";
import { Code, ConnectError } from "@tokuly/api";

/** Errors that will not get better by asking again. */
const FINAL_CODES = new Set([
  Code.NotFound,
  Code.PermissionDenied,
  Code.Unauthenticated,
  Code.InvalidArgument,
  Code.FailedPrecondition,
  Code.AlreadyExists,
  Code.Unimplemented,
]);

export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: 30_000,
        gcTime: 5 * 60_000,
        refetchOnWindowFocus: false,
        retry: (failures, error) =>
          failures < 2 && !(error instanceof ConnectError && FINAL_CODES.has(error.code)),
      },
      mutations: { retry: false },
    },
  });
}
