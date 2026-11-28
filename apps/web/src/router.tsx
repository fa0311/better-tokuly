import { QueryClientProvider } from "@tanstack/react-query";
import { createRouter as createTanStackRouter } from "@tanstack/react-router";
import { getGlobalStartContext } from "@tanstack/react-start";
import { RouteError } from "#/components/route-error";
import { createQueryClient } from "#/lib/query";
import { routeTree } from "./routeTree.gen";

export function getRouter() {
  const queryClient = createQueryClient();
  return createTanStackRouter({
    routeTree,
    context: { queryClient },
    // The nonce of this response's Content-Security-Policy (`src/start.ts`), on the server only.
    ssr: { nonce: getGlobalStartContext()?.nonce },
    scrollRestoration: true,
    defaultPreload: "intent",
    defaultPreloadStaleTime: 0,
    // A page that fails shows so in its place, inside the layout around it.
    defaultErrorComponent: RouteError,
    Wrap: ({ children }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    ),
  });
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof getRouter>;
  }
}
