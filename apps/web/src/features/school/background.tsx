import { useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import { errorReason, school } from "@tokuly/api";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { realtime } from "#/lib/realtime";
import { useSessionStore } from "#/stores/session";
import { keys, refresh, studyClient } from "./api";
import { loadActiveSession, resetActiveSession, setActiveSession, setActiveTimer, useActiveSession } from "./session-store";

/**
 * Runs while someone is signed in: restores their study timer after a
 * reload, keeps telling the server "still here" while it runs (on any page,
 * and in a background tab), follows the timer of the room they are in, and
 * turns the school's realtime events into fresh data and small toasts
 * (a cheer, an invitation to a room, being out of a room).
 */
export default function SchoolBackground() {
  const { t } = useTranslation("school");
  const queryClient = useQueryClient();
  const router = useRouter();
  const sessionId = useActiveSession((state) => state.session?.id);
  const roomId = useActiveSession((state) => state.session?.roomId);
  const heartbeatSeconds = useActiveSession((state) => state.heartbeatSeconds);

  // The timer as the server has it: on sign-in, and whenever the connection comes back.
  useEffect(() => {
    const load = () => loadActiveSession(() => studyClient().getCurrentStudySession({}));
    void load();
    let connectedBefore = false;
    const client = realtime();
    const off = client.onFrame((frame) => {
      if (frame.body.case === "welcome") {
        if (connectedBefore) void load();
        connectedBefore = true;
        return;
      }
      if (frame.body.case !== "school") return;
      const event = frame.body.value.event;
      switch (event.case) {
        case "session": {
          setActiveSession(event.value);
          if (event.value.state === school.SessionState.ENDED) refresh(queryClient, "study", "rooms");
          break;
        }
        case "friendStudying":
          void queryClient.invalidateQueries({ queryKey: keys.studying });
          break;
        case "cheer": {
          toast(t("study.cheered", { name: event.value.from?.displayName ?? "" }), { icon: "📣" });
          void queryClient.invalidateQueries({ queryKey: keys.cheers(event.value.sessionId) });
          const current = useActiveSession.getState().session;
          if (current?.id === event.value.sessionId) useActiveSession.setState({ session: { ...current, cheerCount: current.cheerCount + 1 } });
          break;
        }
        case "roomInvitation": {
          const room = event.value;
          toast(t("rooms.invited", { name: room.host?.displayName ?? "", title: room.title }), {
            icon: "🚪",
            action: { label: t("rooms.open"), onClick: () => void router.navigate({ to: "/school/rooms/$roomId", params: { roomId: room.id } }) },
          });
          refresh(queryClient, "rooms");
          break;
        }
        case "roomLeft": {
          const reason = event.value.reason;
          if (event.value.session) setActiveSession(event.value.session);
          else setActiveSession(undefined);
          toast(
            reason === school.RoomLeaveReason.CLOSED ? t("rooms.left.closed") : reason === school.RoomLeaveReason.REMOVED ? t("rooms.left.removed") : t("rooms.left.idle"),
            { icon: "🚪" },
          );
          refresh(queryClient, "rooms", "study");
          break;
        }
        case "roomTimer": {
          // A room's timer is also the timer of the session spent in it.
          const topicRoom = frame.topic.replace(/^study:/, "");
          setActiveTimer(topicRoom, event.value);
          if (useActiveSession.getState().session?.roomId === topicRoom) void load();
          break;
        }
        default:
      }
    });
    client.start();
    return off;
  }, [queryClient, router, t]);

  // Whoever is in a room hears its timer wherever they are in the app.
  useEffect(() => {
    if (!roomId) return;
    const unsubscribe = realtime().subscribe(`study:${roomId}`);
    realtime().start();
    return unsubscribe;
  }, [roomId]);

  // "Still here": the server counts study time up to the last heartbeat only.
  useEffect(() => {
    if (!sessionId) return;
    let stopped = false;
    const beat = async () => {
      try {
        const response = await studyClient().heartbeatStudySession({});
        if (stopped) return;
        setActiveSession(response.session, response.heartbeatIntervalSeconds);
        if (response.session?.state === school.SessionState.ENDED) {
          if (response.session.autoStopped) toast(t("study.autoStopped"), { icon: "⏱️" });
          refresh(queryClient, "study", "rooms");
        }
      } catch (error) {
        if (!stopped && errorReason(error) === "NO_ACTIVE_SESSION") {
          setActiveSession(undefined);
          refresh(queryClient, "study", "rooms");
        }
      }
    };
    const interval = window.setInterval(beat, Math.max(10, heartbeatSeconds) * 1000);
    // A background tab's timers are slowed down: catch up the moment it is looked at again.
    const onVisible = () => {
      if (document.visibilityState === "visible") void beat();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      stopped = true;
      window.clearInterval(interval);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [sessionId, heartbeatSeconds, queryClient, t]);

  // The timer belongs to the account: it does not outlive the session.
  useEffect(
    () =>
      useSessionStore.subscribe((session) => {
        if (session.state.status === "signedOut") resetActiveSession();
      }),
    [],
  );

  return null;
}
