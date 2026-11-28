import { Suspense } from "react";
import { onDemand } from "#/components/on-demand";
import { useMe } from "#/stores/session";
import { useCompose } from "./compose-store";

const Composer = onDemand(() => import("./compose").then((module) => module.Composer), "toast");

/**
 * The composer of the app. It is mounted once (with the notes background) and
 * opened through `openCompose`; the composer itself loads the first time it opens.
 */
export function ComposeHost() {
  const open = useCompose((state) => state.open);
  const context = useCompose((state) => state.context);
  const me = useMe();
  if (!open || !me?.user) return null;
  return (
    <Suspense fallback={null}>
      <Composer context={context} user={me.user} />
    </Suspense>
  );
}
