import type { QueryClient } from "@tanstack/react-query";
import { type chat, errorReason } from "@tokuly/api";
import type { Room } from "livekit-client";
import { toast } from "sonner";
import { create } from "zustand";
import i18n from "#/lib/i18n";
import { loadLiveKit, sessionWriters, startRoomAudio } from "#/lib/livekit";
import { claimAudioFocus, releaseAudioFocus } from "#/stores/audio-focus";
import { callClient } from "../api";
import { patchConversation, readConversation } from "../conversations";
import { errorMessage } from "../errors";
import { useLive } from "../live";

/**
 * The call this browser is in. There is at most one: its state lives here,
 * its LiveKit room in this module, and the call screen (mounted for the whole
 * app) draws both, so a call survives moving around the app.
 */
export interface ActiveCall {
  /** Empty until the server has created the call. */
  callId: string;
  conversationId: string;
  /** Started as a video call. */
  video: boolean;
  startedByMe: boolean;
  phase: "starting" | "connecting" | "connected" | "reconnecting";
  /** Shrunk to a pill, so the app can be used while talking. */
  minimized: boolean;
  micOn: boolean;
  cameraOn: boolean;
  screenOn: boolean;
  /** The browser refuses to play sound until the person taps. */
  audioBlocked: boolean;
  /** Bumped whenever participants or tracks of the room change. */
  revision: number;
}

export const useCallSession = create<{ active: ActiveCall | null }>(() => ({ active: null }));

let room: Room | null = null;

/** The LiveKit room of the active call, once it exists. */
export function currentRoom(): Room | null {
  return room;
}

const t = (key: string) => i18n.t(key, { ns: "chat" });

const { update, touch } = sessionWriters(useCallSession);

function noteCall(queryClient: QueryClient, call: chat.Call): void {
  useLive.getState().setCall(call);
  patchConversation(queryClient, call.conversationId, (conversation) =>
    conversation.activeCallId === call.id
      ? conversation
      : { ...conversation, activeCallId: call.id },
  );
}

/** Lets go of the room and the devices, and closes the call screen. */
async function close(): Promise<void> {
  const leaving = room;
  room = null;
  useCallSession.setState({ active: null });
  releaseAudioFocus("call");
  await leaving?.disconnect().catch(() => {});
}

async function connect(
  credentials: chat.CallCredentials | undefined,
  video: boolean,
): Promise<void> {
  if (!credentials) throw new Error("no call credentials");
  const { Room, RoomEvent } = await loadLiveKit();
  // The person hung up while the client was loading.
  if (!useCallSession.getState().active) return;
  const session = new Room({ adaptiveStream: true, dynacast: true });
  room = session;
  const mine = () => room === session;
  for (const event of [
    RoomEvent.ParticipantConnected,
    RoomEvent.ParticipantDisconnected,
    RoomEvent.TrackSubscribed,
    RoomEvent.TrackUnsubscribed,
    RoomEvent.TrackMuted,
    RoomEvent.TrackUnmuted,
    RoomEvent.LocalTrackPublished,
    RoomEvent.LocalTrackUnpublished,
    RoomEvent.ActiveSpeakersChanged,
  ] as const) {
    session.on(event, () => mine() && touch());
  }
  session.on(RoomEvent.Reconnecting, () => mine() && update({ phase: "reconnecting" }));
  session.on(RoomEvent.Reconnected, () => mine() && update({ phase: "connected" }));
  session.on(
    RoomEvent.AudioPlaybackStatusChanged,
    () => mine() && update({ audioBlocked: !session.canPlaybackAudio }),
  );
  // Screen sharing can be stopped from the browser's own bar.
  session.on(
    RoomEvent.LocalTrackUnpublished,
    () =>
      mine() &&
      update({
        screenOn: session.localParticipant.isScreenShareEnabled,
        cameraOn: session.localParticipant.isCameraEnabled,
      }),
  );
  session.on(RoomEvent.Disconnected, () => {
    if (!mine()) return;
    // The connection is gone for good (the call ended, or the network did).
    toast(t("call.toast.ended"));
    void close();
  });

  update({ phase: "connecting" });
  await session.connect(credentials.url, credentials.token);
  if (!mine()) return;
  update({ phase: "connected", audioBlocked: !session.canPlaybackAudio });

  try {
    await session.localParticipant.setMicrophoneEnabled(true);
    update({ micOn: true });
  } catch {
    toast.error(t("call.toast.noMicrophone"));
  }
  if (video && mine()) {
    try {
      await session.localParticipant.setCameraEnabled(true);
      update({ cameraOn: true });
    } catch {
      toast.error(t("call.toast.noCamera"));
    }
  }
}

function begin(conversationId: string, video: boolean, startedByMe: boolean, callId = ""): boolean {
  if (useCallSession.getState().active) {
    toast.error(t("call.toast.busy"));
    return false;
  }
  // A call has the microphone to itself: an audio room the person is in pauses until it ends.
  claimAudioFocus({ holder: "call", yield: () => void hangUp() });
  useCallSession.setState({
    active: {
      callId,
      conversationId,
      video,
      startedByMe,
      phase: "starting",
      minimized: false,
      micOn: false,
      cameraOn: false,
      screenOn: false,
      audioBlocked: false,
      revision: 0,
    },
  });
  return true;
}

/** Accepts a ringing call, or joins the call running in a group. */
export async function joinCall(
  queryClient: QueryClient,
  call: Pick<chat.Call, "id" | "conversationId" | "video">,
): Promise<void> {
  if (!begin(call.conversationId, call.video, false, call.id)) return;
  try {
    const response = await callClient().joinCall({ callId: call.id });
    if (response.call) noteCall(queryClient, response.call);
    await connect(response.credentials, call.video);
  } catch (error) {
    toast.error(errorMessage(error));
    await close();
    // Joined on the server but never connected: do not stay in the call as a ghost.
    void callClient()
      .leaveCall({ callId: call.id })
      .catch(() => {});
  }
}

/** Starts a voice or video call in a conversation and rings the others. */
export async function startCall(
  queryClient: QueryClient,
  conversationId: string,
  video: boolean,
): Promise<void> {
  if (!begin(conversationId, video, true)) return;
  try {
    const response = await callClient().startCall({ conversationId, video });
    if (!useCallSession.getState().active) {
      // Hung up before the call existed: take it back.
      if (response.call)
        void callClient()
          .leaveCall({ callId: response.call.id })
          .catch(() => {});
      return;
    }
    if (response.call) {
      update({ callId: response.call.id });
      noteCall(queryClient, response.call);
    }
    await connect(response.credentials, video);
  } catch (error) {
    const started = useCallSession.getState().active?.callId;
    await close();
    // The call exists but could not be connected to: end it rather than let it ring on.
    if (started)
      void callClient()
        .leaveCall({ callId: started })
        .catch(() => {});
    // Somebody else started one in the meantime: join theirs instead.
    const running = readConversation(queryClient, conversationId)?.activeCallId;
    if (errorReason(error) === "CALL_IN_PROGRESS" && running)
      return joinCall(queryClient, { id: running, conversationId, video });
    toast.error(errorMessage(error));
  }
}

/** Says no to a ringing call. */
export async function declineCall(callId: string): Promise<void> {
  try {
    const { call } = await callClient().declineCall({ callId });
    if (call) useLive.getState().setCall(call);
  } catch (error) {
    // Ended or answered elsewhere in the meantime: nothing left to decline.
    if (errorReason(error) !== "CALL_ENDED" && errorReason(error) !== "ALREADY_JOINED")
      toast.error(errorMessage(error));
  }
}

/** Leaves the active call. */
export async function hangUp(): Promise<void> {
  const active = useCallSession.getState().active;
  if (!active) return;
  await close();
  if (active.callId)
    await callClient()
      .leaveCall({ callId: active.callId })
      .catch(() => {});
}

/** The call ended for everyone (an event said so): close the screen without leaving again. */
export function callEnded(callId: string): void {
  if (useCallSession.getState().active?.callId !== callId) return;
  toast(t("call.toast.ended"));
  void close();
}

export function setMinimized(minimized: boolean): void {
  update({ minimized });
}

export async function toggleMicrophone(): Promise<void> {
  const active = useCallSession.getState().active;
  if (!room || !active) return;
  try {
    await room.localParticipant.setMicrophoneEnabled(!active.micOn);
    update({ micOn: !active.micOn });
  } catch {
    toast.error(t("call.toast.noMicrophone"));
  }
}

export async function toggleCamera(): Promise<void> {
  const active = useCallSession.getState().active;
  if (!room || !active) return;
  try {
    await room.localParticipant.setCameraEnabled(!active.cameraOn);
    update({ cameraOn: !active.cameraOn });
  } catch {
    toast.error(t("call.toast.noCamera"));
  }
}

/** Whether this browser can share the screen at all (phones cannot). */
export function canShareScreen(): boolean {
  return (
    typeof navigator !== "undefined" &&
    typeof navigator.mediaDevices?.getDisplayMedia === "function"
  );
}

export async function toggleScreenShare(): Promise<void> {
  const active = useCallSession.getState().active;
  if (!room || !active) return;
  try {
    await room.localParticipant.setScreenShareEnabled(!active.screenOn);
    update({ screenOn: room.localParticipant.isScreenShareEnabled });
  } catch {
    // The person closed the browser's picker: not an error worth a toast.
    update({ screenOn: room.localParticipant.isScreenShareEnabled });
  }
}

/** After a tap: lets the browser play the others' sound. */
export async function unblockAudio(): Promise<void> {
  update({ audioBlocked: await startRoomAudio(() => room) });
}
