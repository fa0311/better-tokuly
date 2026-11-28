import { toMillis } from "@tokuly/api";
import { ringsFor } from "@tokuly/core/chat";
import { Suspense } from "react";
import { onDemand } from "#/components/on-demand";
import { useMe } from "#/stores/session";
import { useLive } from "../live";
import { useCallSession } from "./session";

const IncomingCall = onDemand(() =>
  import("./incoming-call").then((module) => module.IncomingCall),
);
const CallScreen = onDemand(
  () => import("./call-screen").then((module) => module.CallScreen),
  "toast",
);

/**
 * Calls, wherever the person is in the app: the sheet that rings for an
 * incoming call and the screen of the call they are in. Mounted once for the
 * signed-in app.
 */
export function CallHost() {
  const myId = useMe()?.user?.id ?? "";
  const calls = useLive((state) => state.calls);
  const active = useCallSession((state) => state.active);
  // One call rings at a time: the one that started first.
  const ringing = Object.values(calls)
    .filter((call) => ringsFor(call, myId) && call.id !== active?.callId)
    .sort((a, b) => toMillis(a.startedAt) - toMillis(b.startedAt))[0];

  return (
    <>
      <Suspense fallback={null}>
        {ringing && <IncomingCall key={ringing.id} call={ringing} myId={myId} />}
      </Suspense>
      <Suspense fallback={null}>{active && <CallScreen active={active} />}</Suspense>
    </>
  );
}
