import { toMillis } from "@tokuly/api";
import { ringsFor } from "@tokuly/core/chat";
import { lazy, Suspense } from "react";
import { useMe } from "#/stores/session";
import { useLive } from "../live";
import { useCallSession } from "./session";

const IncomingCall = lazy(() =>
  import("./incoming-call").then((module) => ({ default: module.IncomingCall })),
);
const CallScreen = lazy(() =>
  import("./call-screen").then((module) => ({ default: module.CallScreen })),
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
