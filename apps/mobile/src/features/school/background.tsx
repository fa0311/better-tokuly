import { useQueryClient } from "@tanstack/react-query";
import { school } from "@tokuly/api";
import { refreshSchool, schoolKeys } from "@tokuly/core/school";
import { useEffect } from "react";
import { Alert, AppState } from "react-native";
import { currentAccount } from "#/features/session/account";
import i18n from "#/lib/i18n";
import { realtime } from "#/lib/realtime";
import {
  followAccount,
  forgetReminder,
  heartbeat,
  loadSession,
  remindBeforeIdle,
  seenAutoStop,
  takeRoomTimer,
  takeSession,
  timerAccount,
  useStudyTimer,
} from "./session";

const Leave = school.RoomLeaveReason;

/**
 * Runs while someone is signed in: restores the study timer, keeps telling
 * the server "still here" while the app is open, reminds before the server
 * stops a timer left behind in the background, follows the room the person
 * studies in, and keeps friends who are studying current.
 */
export default function SchoolBackground() {
  const queryClient = useQueryClient();
  const sessionId = useStudyTimer((state) => state.session?.id);
  const roomId = useStudyTimer((state) => state.session?.roomId);
  const heartbeatSeconds = useStudyTimer((state) => state.heartbeatSeconds);
  const autoStopped = useStudyTimer((state) => state.autoStopped);

  // The timer as the server has it: on sign-in, and whenever the connection comes back.
  useEffect(() => {
    const account = currentAccount();
    if (!account) return;
    followAccount(account);
    void loadSession(account);
    let welcomed = false;
    const client = realtime();
    const off = client.onFrame((frame) => {
      if (frame.body.case === "welcome") {
        if (welcomed) {
          void loadSession(account);
          void queryClient.invalidateQueries({ queryKey: schoolKeys.studying });
        }
        welcomed = true;
        return;
      }
      if (frame.body.case !== "school") return;
      const event = frame.body.value.event;
      switch (event.case) {
        case "session":
          takeSession(event.value);
          if (event.value.state === school.SessionState.ENDED)
            refreshSchool(queryClient, "study", "rooms");
          return;
        case "friendStudying":
          void queryClient.invalidateQueries({ queryKey: schoolKeys.studying });
          return;
        case "cheer": {
          void queryClient.invalidateQueries({
            queryKey: schoolKeys.cheers(event.value.sessionId),
          });
          const { session } = useStudyTimer.getState();
          if (session?.id === event.value.sessionId)
            useStudyTimer.setState({ session: { ...session, cheerCount: session.cheerCount + 1 } });
          return;
        }
        case "roomLeft": {
          takeSession(event.value.session);
          const reason = event.value.reason;
          Alert.alert(
            i18n.t(
              reason === Leave.CLOSED
                ? "school:rooms.left.closed"
                : reason === Leave.REMOVED
                  ? "school:rooms.left.removed"
                  : "school:rooms.left.idle",
            ),
          );
          refreshSchool(queryClient, "rooms", "study");
          return;
        }
        case "roomTimer": {
          // A room's timer is also the timer of the session spent in it.
          const topicRoom = frame.topic.replace(/^study:/, "");
          takeRoomTimer(topicRoom, event.value);
          if (useStudyTimer.getState().session?.roomId === topicRoom) void loadSession(account);
          return;
        }
        default:
      }
    });
    client.start();
    return off;
  }, [queryClient]);

  // Whoever is in a room hears its timer wherever they are in the app.
  useEffect(() => {
    if (!roomId) return;
    const { release } = realtime().subscribe(`study:${roomId}`);
    realtime().start();
    return release;
  }, [roomId]);

  // "Still here" while the app is open. Off screen the phone runs no timers:
  // a last heartbeat on the way out, a reminder before the server's limit,
  // and a heartbeat the moment the app is back (which counts the time between).
  useEffect(() => {
    if (!sessionId) return;
    const beat = () => {
      const account = timerAccount();
      if (account) void heartbeat(account);
    };
    let interval: ReturnType<typeof setInterval> | undefined;
    const onScreen = () => {
      clearInterval(interval);
      interval = setInterval(beat, Math.max(10, heartbeatSeconds) * 1000);
    };
    if (AppState.currentState === "active") onScreen();
    const appState = AppState.addEventListener("change", (state) => {
      if (state === "active") {
        void forgetReminder();
        beat();
        onScreen();
      } else if (state === "background") {
        clearInterval(interval);
        beat();
        void remindBeforeIdle();
      }
    });
    return () => {
      clearInterval(interval);
      appState.remove();
    };
  }, [sessionId, heartbeatSeconds]);

  useEffect(() => {
    if (!autoStopped) return;
    seenAutoStop();
    refreshSchool(queryClient, "study", "rooms");
    Alert.alert(i18n.t("school:study.autoStopped"));
  }, [autoStopped, queryClient]);

  return null;
}
