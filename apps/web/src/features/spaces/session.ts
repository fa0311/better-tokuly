import type { QueryClient } from "@tanstack/react-query";
import { errorReason, isNotFound, spaces } from "@tokuly/api";
import type { DisconnectReason, Room } from "livekit-client";
import { toast } from "sonner";
import { create } from "zustand";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";
import {
  hangoutClient,
  keys,
  patchHangout,
  patchSpace,
  rememberHangout,
  rememberSpace,
  spaceClient,
} from "./api";
import { onStage } from "./model";

/**
 * The room or hangout this browser is inside. There is at most one (the
 * server holds a person to one, too): its state lives in this store and its
 * LiveKit connection in this module, so the audio keeps going while the
 * person moves around the app, and the room screen and the mini player are
 * two views of the same thing.
 */
export interface ActiveVenue {
  kind: "space" | "hangout";
  id: string;
  /** The room's title or the hangout's name, for the mini player. */
  title: string;
  /** The hangout's emoji; empty for rooms. */
  emoji: string;
  phase: "connecting" | "connected" | "reconnecting";
  /** The connection may send a microphone (LiveKit's word, which follows the role). */
  canPublish: boolean;
  micOn: boolean;
  /** The browser refuses to play sound until the person taps. */
  audioBlocked: boolean;
  /** Bumped whenever who is talking or muted changes. */
  revision: number;
}

/**
 * Why the person is not inside any more although they did not leave:
 * - `removed` / `banned`: a moderator's doing;
 * - `ended`: the room is over; `closed`: the hangout is gone (or they are no member any more);
 * - `elsewhere`: they entered with another device or tab;
 * - `lost`: the connection could not be brought back.
 */
export type ExitReason = "removed" | "banned" | "ended" | "closed" | "elsewhere" | "lost";

export interface Exit {
  kind: "space" | "hangout";
  id: string;
  reason: ExitReason;
}

interface SessionState {
  active: ActiveVenue | null;
  /** The last involuntary exit, until the person acknowledges it or enters something again. */
  exit: Exit | null;
}

export const useAudioSession = create<SessionState>(() => ({ active: null, exit: null }));

const t = (key: string, options?: Record<string, unknown>) =>
  i18n.t(key, { ns: "spaces", ...options });
const fail = (error: unknown) => toast.error(errorMessage(error, { namespace: "spaces" }));

let room: Room | null = null;
/** Counts every entering and leaving, so work that finishes late can tell it is stale. */
let generation = 0;
/** Automatic re-entries since the connection was last stable. */
let rejoins = 0;
let stableTimer: ReturnType<typeof setTimeout> | undefined;
let promoteTimer: ReturnType<typeof setTimeout> | undefined;

/** When each person was last heard talking, by user ID. */
const lastSpoke = new Map<string, number>();
let quietTimer: ReturnType<typeof setTimeout> | undefined;
/** A face stays lit this long after its voice went quiet, so it does not flicker between words. */
const SPEAKING_LINGER_MS = 1200;

const MAX_REJOINS = 2;
const MIC_KEY = "tokuly.spaces.microphone";
const SPEAKER_KEY = "tokuly.spaces.speaker";

function stored(key: string): string | undefined {
  try {
    return window.localStorage.getItem(key) || undefined;
  } catch {
    return undefined;
  }
}

function store(key: string, value: string): void {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Private windows may refuse: the choice then lasts for this visit.
  }
}

function update(change: Partial<ActiveVenue>): void {
  useAudioSession.setState((state) =>
    state.active ? { active: { ...state.active, ...change } } : state,
  );
}

function touch(): void {
  useAudioSession.setState((state) =>
    state.active ? { active: { ...state.active, revision: state.active.revision + 1 } } : state,
  );
}

/** The hidden element the others' voices play through, kept outside React so navigation never interrupts them. */
function sink(): HTMLElement {
  let element = document.getElementById("spaces-audio");
  if (!element) {
    element = document.createElement("div");
    element.id = "spaces-audio";
    element.hidden = true;
    document.body.appendChild(element);
  }
  return element;
}

/** Lets go of the connection and the microphone. */
async function close(exit: Exit | null = null): Promise<void> {
  generation++;
  clearTimeout(stableTimer);
  clearTimeout(promoteTimer);
  clearTimeout(quietTimer);
  lastSpoke.clear();
  const leaving = room;
  room = null;
  useAudioSession.setState({ active: null, exit });
  document.getElementById("spaces-audio")?.replaceChildren();
  await leaving?.disconnect().catch(() => {});
}

function begin(venue: Pick<ActiveVenue, "kind" | "id" | "title" | "emoji">): number {
  rejoins = 0;
  useAudioSession.setState({
    active: {
      ...venue,
      phase: "connecting",
      canPublish: false,
      micOn: false,
      audioBlocked: false,
      revision: 0,
    },
    exit: null,
  });
  return generation;
}

async function connect(
  credentials: spaces.AudioCredentials | undefined,
  mine: number,
  options: { microphone?: boolean } = {},
): Promise<void> {
  if (!credentials) throw new Error("no audio credentials");
  const { Room, RoomEvent, Track } = await import("livekit-client");
  // The person left while the client was loading.
  if (mine !== generation) return;
  const session = new Room({
    audioCaptureDefaults: {
      deviceId: stored(MIC_KEY),
      echoCancellation: true,
      noiseSuppression: true,
      autoGainControl: true,
    },
    audioOutput: { deviceId: stored(SPEAKER_KEY) },
  });
  const previous = room;
  room = session;
  void previous?.disconnect().catch(() => {});
  sink().replaceChildren();
  const current = () => room === session && mine === generation;

  for (const event of [
    RoomEvent.ParticipantConnected,
    RoomEvent.ParticipantDisconnected,
    RoomEvent.TrackMuted,
    RoomEvent.TrackUnmuted,
    RoomEvent.TrackPublished,
    RoomEvent.TrackUnpublished,
    RoomEvent.ActiveSpeakersChanged,
  ] as const) {
    session.on(event, () => current() && touch());
  }
  session.on(RoomEvent.ActiveSpeakersChanged, (speakers) => {
    if (!current()) return;
    const now = Date.now();
    for (const speaker of speakers) lastSpoke.set(speaker.identity, now);
    // Looked at again once the last voice has been quiet for the linger.
    clearTimeout(quietTimer);
    quietTimer = setTimeout(() => current() && touch(), SPEAKING_LINGER_MS + 50);
  });
  session.on(RoomEvent.TrackSubscribed, (track) => {
    if (!current() || track.kind !== Track.Kind.Audio) return;
    sink().appendChild(track.attach());
    touch();
  });
  session.on(RoomEvent.TrackUnsubscribed, (track) => {
    for (const element of track.detach()) element.remove();
    if (current()) touch();
  });
  // A moderator's mute, a demotion, or the person's own tap: the button follows the track.
  for (const event of [
    RoomEvent.LocalTrackPublished,
    RoomEvent.LocalTrackUnpublished,
    RoomEvent.TrackMuted,
    RoomEvent.TrackUnmuted,
  ] as const) {
    session.on(
      event,
      () => current() && update({ micOn: session.localParticipant.isMicrophoneEnabled }),
    );
  }
  session.on(RoomEvent.ParticipantPermissionsChanged, (_previous, participant) => {
    if (!current() || participant !== session.localParticipant) return;
    const canPublish = Boolean(session.localParticipant.permissions?.canPublish);
    update({ canPublish, micOn: canPublish && session.localParticipant.isMicrophoneEnabled });
  });
  session.on(RoomEvent.Reconnecting, () => current() && update({ phase: "reconnecting" }));
  session.on(RoomEvent.SignalReconnecting, () => current() && update({ phase: "reconnecting" }));
  session.on(RoomEvent.Reconnected, () => current() && update({ phase: "connected" }));
  session.on(
    RoomEvent.AudioPlaybackStatusChanged,
    () => current() && update({ audioBlocked: !session.canPlaybackAudio }),
  );
  session.on(RoomEvent.Disconnected, (reason) => {
    if (current()) void disconnected(mine, reason);
  });

  await session.connect(credentials.url, credentials.token);
  if (!current()) {
    void session.disconnect().catch(() => {});
    return;
  }
  update({
    phase: "connected",
    canPublish: session.localParticipant.permissions?.canPublish ?? credentials.canPublish,
    audioBlocked: !session.canPlaybackAudio,
  });
  clearTimeout(stableTimer);
  stableTimer = setTimeout(() => {
    rejoins = 0;
  }, 60_000);
  if (options.microphone) await setMicrophone(true);
}

/** Fresh credentials for whatever the person holds now, from the venue they are inside. */
async function freshCredentials(
  queryClient: QueryClient,
  venue: Pick<ActiveVenue, "kind" | "id">,
): Promise<spaces.AudioCredentials | undefined> {
  if (venue.kind === "space") {
    const response = await spaceClient().joinSpace({ spaceId: venue.id });
    rememberSpace(queryClient, response.space);
    return response.credentials;
  }
  const response = await hangoutClient().joinHangout({ hangoutId: venue.id });
  rememberHangout(queryClient, response.hangout);
  return response.credentials;
}

let client: QueryClient | null = null;

/**
 * LiveKit gave the connection up. Before anything is tried again the server
 * is asked what is true: someone who was removed, banned or left behind by a
 * room that ended lands on that state and is not reconnected.
 */
async function disconnected(mine: number, reason: DisconnectReason | undefined): Promise<void> {
  const active = useAudioSession.getState().active;
  if (!active || !client) return;
  const { DisconnectReason } = await import("livekit-client");
  if (mine !== generation) return;
  const venue = { kind: active.kind, id: active.id };
  if (reason === DisconnectReason.DUPLICATE_IDENTITY)
    return void close({ ...venue, reason: "elsewhere" });
  const cut =
    reason === DisconnectReason.PARTICIPANT_REMOVED || reason === DisconnectReason.ROOM_DELETED;
  update({ phase: "reconnecting" });
  try {
    const exit = await whyOutside(client, venue, cut);
    if (mine !== generation) return;
    if (exit) return void close({ ...venue, reason: exit });
    if (rejoins >= MAX_REJOINS) throw new Error("gave up");
    rejoins++;
    const microphone = active.micOn;
    const credentials = await freshCredentials(client, venue);
    if (mine !== generation) return;
    await connect(credentials, mine, { microphone });
  } catch (error) {
    if (mine !== generation) return;
    // Entering again was refused: the room is gone for this person, or full, or the network is.
    const gone = isNotFound(error) || errorReason(error) === "BANNED_FROM_SPACE";
    void close({ ...venue, reason: gone ? "removed" : "lost" });
  }
}

/** Asks the server whether the person is still inside; the reason they are not, otherwise. */
async function whyOutside(
  queryClient: QueryClient,
  venue: Pick<ActiveVenue, "kind" | "id">,
  cut: boolean,
): Promise<ExitReason | null> {
  try {
    if (venue.kind === "space") {
      const { space } = await spaceClient().getSpace({ spaceId: venue.id });
      rememberSpace(queryClient, space);
      if (!space) return "removed";
      if (space.state !== spaces.SpaceState.LIVE) return "ended";
      if (!space.viewer?.inside) return cut ? "removed" : "lost";
      return null;
    }
    const { hangout } = await hangoutClient().getHangout({ hangoutId: venue.id });
    rememberHangout(queryClient, hangout);
    if (hangout?.viewer?.state !== spaces.HangoutMemberState.MEMBER) return "closed";
    if (!hangout.viewer.inside) return "lost";
    return null;
  } catch (error) {
    if (isNotFound(error)) return venue.kind === "space" ? "removed" : "closed";
    throw error;
  }
}

/** Enters a live room as whatever the person is there (host, co-host, listener). */
export async function enterSpace(
  queryClient: QueryClient,
  space: Pick<spaces.Space, "id" | "title">,
): Promise<boolean> {
  client = queryClient;
  await close();
  const mine = begin({ kind: "space", id: space.id, title: space.title, emoji: "" });
  try {
    const response = await spaceClient().joinSpace({ spaceId: space.id });
    rememberSpace(queryClient, response.space);
    if (response.space) update({ title: response.space.title });
    await connect(response.credentials, mine);
    return mine === generation;
  } catch (error) {
    if (mine !== generation) return false;
    fail(error);
    await close();
    if (isNotFound(error)) queryClient.setQueryData(keys.space(space.id), null);
    else void queryClient.invalidateQueries({ queryKey: keys.space(space.id) });
    // Entered on the server but never connected: do not stay inside as a ghost.
    void spaceClient()
      .leaveSpace({ spaceId: space.id })
      .catch(() => {});
    return false;
  }
}

/** Connects to a room the server just put the person in (created on the spot, or started). */
export async function adoptSpace(
  queryClient: QueryClient,
  space: spaces.Space,
  credentials: spaces.AudioCredentials | undefined,
): Promise<void> {
  client = queryClient;
  await close();
  const mine = begin({ kind: "space", id: space.id, title: space.title, emoji: "" });
  rememberSpace(queryClient, space);
  try {
    await connect(credentials, mine);
  } catch {
    if (mine !== generation) return;
    // The room exists and is the person's: they can come back in from its page.
    toast.error(t("toast.connectFailed"));
    await close();
  }
}

/** Enters a hangout's voice room; everyone inside can talk, so the microphone opens. */
export async function enterHangout(
  queryClient: QueryClient,
  hangout: Pick<spaces.Hangout, "id" | "name" | "emoji">,
): Promise<boolean> {
  client = queryClient;
  await close();
  const mine = begin({
    kind: "hangout",
    id: hangout.id,
    title: hangout.name,
    emoji: hangout.emoji,
  });
  try {
    const response = await hangoutClient().joinHangout({ hangoutId: hangout.id });
    rememberHangout(queryClient, response.hangout);
    await connect(response.credentials, mine, { microphone: true });
    return mine === generation;
  } catch (error) {
    if (mine !== generation) return false;
    fail(error);
    await close();
    void queryClient.invalidateQueries({ queryKey: keys.hangout(hangout.id) });
    void hangoutClient()
      .leaveHangout({ hangoutId: hangout.id })
      .catch(() => {});
    return false;
  }
}

/** Leaves whatever the person is inside, quietly. */
export async function leaveVenue(queryClient: QueryClient): Promise<void> {
  const active = useAudioSession.getState().active;
  if (!active) return;
  await close();
  try {
    if (active.kind === "space") {
      patchSpace(queryClient, active.id, (space) =>
        space.viewer
          ? { ...space, viewer: { ...space.viewer, inside: false, handRaised: false } }
          : space,
      );
      await spaceClient().leaveSpace({ spaceId: active.id });
      void queryClient.invalidateQueries({ queryKey: keys.space(active.id) });
    } else {
      await hangoutClient().leaveHangout({ hangoutId: active.id });
      patchHangout(queryClient, active.id, (hangout) =>
        hangout.viewer ? { ...hangout, viewer: { ...hangout.viewer, inside: false } } : hangout,
      );
    }
  } catch {
    // Gone already (the room ended, the hangout closed): there is nothing left to leave.
  }
}

let endingMyself = "";

/** The person is about to end this room themselves: its end is then no news to them. */
export function expectEnd(spaceId: string): void {
  endingMyself = spaceId;
}

/** The server said the person is outside (an event): close without leaving again. */
export function putOutside(kind: ActiveVenue["kind"], id: string, reason: ExitReason): void {
  const { active, exit } = useAudioSession.getState();
  if (active?.kind === kind && active.id === id) {
    const expected = kind === "space" && reason === "ended" && endingMyself === id;
    return void close(expected ? null : { kind, id, reason });
  }
  // The connection was cut before the event said why: a moderator's reason replaces the guess.
  if (exit?.kind === kind && exit.id === id && (reason === "removed" || reason === "banned"))
    useAudioSession.setState({ exit: { kind, id, reason } });
}

/** The person has seen why they are outside. */
export function acknowledgeExit(): void {
  useAudioSession.setState({ exit: null });
}

/**
 * The person's role in the room changed. LiveKit switches the open
 * connection's permissions by itself; should that not have happened a moment
 * later, the connection is replaced by one made with a token for the new role.
 */
export function roleChanged(queryClient: QueryClient, spaceId: string, role: spaces.SpaceRole) {
  const active = useAudioSession.getState().active;
  if (!room || active?.kind !== "space" || active.id !== spaceId) return;
  client = queryClient;
  clearTimeout(promoteTimer);
  const stage = onStage(role);
  if (!stage) {
    // Back in the audience: the microphone goes at once, whatever LiveKit has heard so far.
    void room.localParticipant.setMicrophoneEnabled(false).catch(() => {});
    update({ micOn: false });
  }
  const mine = generation;
  promoteTimer = setTimeout(() => {
    if (mine !== generation || !room) return;
    if (Boolean(room.localParticipant.permissions?.canPublish) !== stage)
      void reconnectWithFreshToken(queryClient);
  }, 4000);
}

async function reconnectWithFreshToken(queryClient: QueryClient): Promise<boolean> {
  const active = useAudioSession.getState().active;
  if (!active) return false;
  const mine = generation;
  try {
    const credentials = await freshCredentials(queryClient, active);
    if (mine !== generation) return false;
    await connect(credentials, mine);
    return mine === generation;
  } catch (error) {
    if (mine === generation) fail(error);
    return false;
  }
}

async function setMicrophone(on: boolean): Promise<void> {
  if (!room) return;
  try {
    await room.localParticipant.setMicrophoneEnabled(on);
    update({ micOn: room.localParticipant.isMicrophoneEnabled });
  } catch {
    update({ micOn: false });
    toast.error(t("toast.noMicrophone"));
  }
}

/** Mutes or unmutes the person's own microphone. */
export async function toggleMicrophone(queryClient: QueryClient): Promise<void> {
  const active = useAudioSession.getState().active;
  if (!room || !active) return;
  if (!active.micOn && !room.localParticipant.permissions?.canPublish) {
    // On stage by the server's word, but this connection still holds a listener's permissions.
    if (!(await reconnectWithFreshToken(queryClient))) return;
  }
  await setMicrophone(!active.micOn);
}

/** After a tap: lets the browser play the others' sound. */
export async function unblockAudio(): Promise<void> {
  await room?.startAudio().catch(() => {});
  update({ audioBlocked: room ? !room.canPlaybackAudio : false });
}

export interface AudioDevices {
  microphones: MediaDeviceInfo[];
  speakers: MediaDeviceInfo[];
  microphoneId: string;
  speakerId: string;
}

/** The microphones and speakers this browser offers, and the ones in use. */
export async function listAudioDevices(): Promise<AudioDevices> {
  const { Room } = await import("livekit-client");
  const [microphones, speakers] = await Promise.all([
    Room.getLocalDevices("audioinput", false).catch(() => []),
    Room.getLocalDevices("audiooutput", false).catch(() => []),
  ]);
  return {
    microphones,
    speakers,
    microphoneId: room?.getActiveDevice("audioinput") ?? stored(MIC_KEY) ?? "default",
    speakerId: room?.getActiveDevice("audiooutput") ?? stored(SPEAKER_KEY) ?? "default",
  };
}

/** Switches the microphone or the speaker, now and for the next rooms. */
export async function chooseAudioDevice(
  kind: "audioinput" | "audiooutput",
  deviceId: string,
): Promise<void> {
  store(kind === "audioinput" ? MIC_KEY : SPEAKER_KEY, deviceId);
  try {
    await room?.switchActiveDevice(kind, deviceId);
  } catch {
    toast.error(t("toast.deviceFailed"));
  }
}

/** Who can be heard right now, by user ID. */
export interface Voices {
  /** Talking at this moment. */
  speaking: ReadonlySet<string>;
  /** Connected with an open microphone. */
  open: ReadonlySet<string>;
  /** Connected with audio (on stage, or in the hangout's voice room). */
  connected: ReadonlySet<string>;
}

const SILENCE: Voices = { speaking: new Set(), open: new Set(), connected: new Set() };

/**
 * The voices of the venue the person is inside (`kind` and `id` name it);
 * nothing while they are outside, where there is no connection to read from.
 */
export function useVoices(kind: ActiveVenue["kind"], id: string): Voices {
  const active = useAudioSession((state) =>
    state.active?.kind === kind && state.active.id === id ? state.active : null,
  );
  if (!active || active.phase === "connecting") return SILENCE;
  // The revision is what changes when the connection does; handing it over keeps the read from
  // being memoized away (the connection itself lives outside React).
  return readVoices(active.revision);
}

/** Who can be heard on the open connection, as of `revision`. */
function readVoices(_revision: number): Voices {
  if (!room) return SILENCE;
  const speaking = new Set<string>();
  const open = new Set<string>();
  const connected = new Set<string>();
  const now = Date.now();
  for (const participant of [room.localParticipant, ...room.remoteParticipants.values()]) {
    connected.add(participant.identity);
    if (participant.isMicrophoneEnabled) open.add(participant.identity);
    const heard =
      participant.isSpeaking ||
      now - (lastSpoke.get(participant.identity) ?? 0) < SPEAKING_LINGER_MS;
    if (heard && participant.isMicrophoneEnabled) speaking.add(participant.identity);
  }
  return { speaking, open, connected };
}

/** Whether the person is inside this very venue with this browser. */
export function useIsInside(kind: ActiveVenue["kind"], id: string): boolean {
  return useAudioSession((state) => state.active?.kind === kind && state.active.id === id);
}
