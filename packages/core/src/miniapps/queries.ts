import * as dev from "@tokuly/api/gen/tokuly/dev/v1/dev_pb.js";
import type { ClientSource } from "../query";

type MiniApps = ClientSource<typeof dev.MiniAppService>;

/** Query keys of mini apps; all start with `miniapps`. */
export const miniAppKeys = {
  all: ["miniapps"] as const,
  /** An approved mini app as the caller opens it, with what they granted. */
  app: (appId: string) => ["miniapps", "app", appId] as const,
  scopes: ["miniapps", "scopes"] as const,
  /** The manifest and review of an application the caller owns. */
  review: (appId: string) => ["miniapps", "review", appId] as const,
  /** Staff: manifests waiting for review. */
  queue: ["miniapps", "queue"] as const,
};

export const miniAppQueries = {
  /** Never retried: a mini app that is not there is not coming back on its own. */
  app: (client: MiniApps, appId: string) => ({
    queryKey: miniAppKeys.app(appId),
    queryFn: () => client().getMiniApp({ appId }),
    retry: false,
    staleTime: 60_000,
  }),
  scopes: (client: MiniApps) => ({
    queryKey: miniAppKeys.scopes,
    queryFn: async () => (await client().listMiniAppScopes({})).scopes,
    staleTime: 10 * 60_000,
  }),
};

export { dev };
