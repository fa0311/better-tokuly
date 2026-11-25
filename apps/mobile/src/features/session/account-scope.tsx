import { QueryClientProvider } from "@tanstack/react-query";
import { type ReactNode, useEffect, useState } from "react";
import { createQueryClient } from "#/lib/query";

/**
 * Everything cached for one account. Keyed by the session's generation in
 * the root layout: another account starts with an empty cache.
 */
export function AccountScope({ children }: { children: ReactNode }) {
  const [queryClient] = useState(createQueryClient);
  useEffect(() => () => queryClient.clear(), [queryClient]);
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
}
