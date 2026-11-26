import { AndroidAudioTypePresets, AudioSession, registerGlobals } from "@livekit/react-native";
import { Room } from "livekit-client";

/**
 * The phone's side of LiveKit: WebRTC's globals, registered the first time
 * a room opens (not at start-up), and the system's audio session around each
 * connection — voice chat with the loudspeaker as the default output.
 */

let registered = false;

/** A new room, its audio session started. */
export async function openRoom(): Promise<Room> {
  if (!registered) {
    registerGlobals();
    registered = true;
  }
  await AudioSession.configureAudio({
    android: {
      preferredOutputList: ["bluetooth", "headset", "speaker"],
      audioTypeOptions: AndroidAudioTypePresets.communication,
    },
    ios: { defaultOutput: "speaker" },
  });
  await AudioSession.startAudioSession();
  return new Room({ adaptiveStream: false, dynacast: false });
}

/** Lets go of a room and, with it, the audio session. */
export async function closeRoom(room: Room): Promise<void> {
  await room.disconnect().catch(() => {});
  await AudioSession.stopAudioSession().catch(() => {});
}
