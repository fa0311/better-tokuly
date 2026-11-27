import type { QueryClient } from "@tanstack/react-query";
import { errorReason, isNotFound, spaces } from "@tokuly/api";
import {
  type Exit,
  type ExitReason,
  goneReason,
  markLeftHangout,
  markLeftSpace,
  onStage,
  rememberHangout,
  rememberSpace,
  SILENCE,
  SPEAKING_LINGER_MS,
  spacesKeys,
  type Voices,
  voicesOf,
  whyOutsideHangout,
  whyOutsideSpace,
} from "@tokuly/core/spaces";
import { DisconnectReason, type Room, RoomEvent } from "livekit-client";
import { Alert } from "react-native";
import { create } from "zustand";
import { type Account, currentAccount, isActive } from "#/features/session/account";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";
import { closeRoom, openRoom } from "./livekit";
import { microphoneAllowed } from "./microphone";

/**
 * The room or hangout this phone is inside. There is at most one (the server
 * holds a person to one, too): its state lives in this store and its LiveKit
 * connection in this module, so the sound goes on while the person moves
 * around the app. It belongs to the account that entered: every request is
 * made with that account's API, and it ends before the account leaves the
 * device (`lifecycle.ts`).
 */
export interface ActiveVenue {
  kind: "space" | "hangout";
  id: string;
  /** The room's title or the hangout's name. */
  title: string;
  /** The hangout's emoji; empty for rooms. */
  emoji: string;
  phase: "connecting" | "connected" | "reconnecting";
  /** The role held in a room, as this session last heard; hangouts: speaker. */
  role: spaces.SpaceRole;
  /** The connection may send the microphone (LiveKit's word, which follows the role). */
  canPublish: boolean;
  micOn: boolean;
  /** Bumped whenever who is talking or muted changes. */
  revision: number;
}

interface SessionState {
  active: ActiveVenue | null;
  /** The last involuntary exit, until the person has seen it or enters again. */
  exit: Exit | null;
}

export const useAudioSession = create<SessionState>(() => ({ active: null, exit: null }));

const t = (key: string, options?: Record<string, unknown>) =>
  i18n.t(key, { ns: "spaces", ...options });
const fail = (error: unknown) => Alert.alert(errorMessage(error, { namespace: "spaces" }));

let room: Room | null = null;
/** Whose venue it is: the account that entered. */
let owner: Account | null = null;
/** Counts every entering and leaving, so work that finishes late can tell it is stale. */
let generation = 0;
let promoteTimer: ReturnType<typeof setTimeout> | undefined;

/** When each person was last heard talking, by user ID. */
const lastSpoke = new Map<string, number>();
let quietTimer: ReturnType<typeof setTimeout> | undefined;
/** Tries to get back in after LiveKit gave a connection up, before calling it lost. */
const MAX_REJOINS = 4;

/**
 * Changes the venue's state. Every change bumps the revision: what the
 * connection says (who talks, whose microphone is open) is read again, the
 * person's own microphone included.
 */
function update(patch: Partial<ActiveVenue>): void {
  const active = useAudioSession.getState().active;
  if (active)
    useAudioSession.setState({
      active: { ...active, ...patch, revision: active.revision + 1 },
    });
}

const touch = () => update({});

const spaceService = (account: Account) => account.api.service(spaces.SpaceService);
const hangoutService = (account: Account) => account.api.service(spaces.HangoutService);

/** Lets go of the connection and the microphone. */
async function close(exit: Exit | null = null): Promise<void> {
  generation++;
  clearTimeout(promoteTimer);
  clearTimeout(quietTimer);
  lastSpoke.clear();
  const leaving = room;
  room = null;
  owner = null;
  useAudioSession.setState({ active: null, exit });
  if (leaving) await closeRoom(leaving);
}

function begin(
  account: Account,
  venue: Pick<ActiveVenue, "kind" | "id" | "title" | "emoji">,
  role: spaces.SpaceRole,
): number {
  owner = account;
  useAudioSession.setState({
    active: { ...venue, role, phase: "connecting", canPublish: false, micOn: false, revision: 0 },
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
  const previous = room;
  room = null;
  if (previous) await closeRoom(previous);
  if (mine !== generation) return;
  const session = await openRoom();
  if (mine !== generation) return void closeRoom(session);
  room = session;
  const current = () => room === session && mine === generation;

  for (const event of [
    RoomEvent.ParticipantConnected,
    RoomEvent.ParticipantDisconnected,
    RoomEvent.TrackMuted,
    RoomEvent.TrackUnmuted,
    RoomEvent.TrackPublished,
    RoomEvent.TrackUnpublished,
    RoomEvent.TrackSubscribed,
    RoomEvent.TrackUnsubscribed,
  ] as const) {
    session.on(event, () => current() && touch());
  }
  session.on(RoomEvent.ActiveSpeakersChanged, (speakers) => {
    if (!current()) return;
    const now = Date.now();
    for (const speaker of speakers) lastSpoke.set(speaker.identity, now);
    touch();
    // Looked at again once the last voice has been quiet for the linger.
    clearTimeout(quietTimer);
    quietTimer = setTimeout(() => current() && touch(), SPEAKING_LINGER_MS + 50);
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
  session.on(RoomEvent.Reconnected, () => current() && update({ phase: "connected" }));
  session.on(RoomEvent.Disconnected, (reason) => {
    if (current()) void disconnected(mine, reason);
  });

  await session.connect(credentials.url, credentials.token);
  if (!current()) return void closeRoom(session);
  const canPublish = session.localParticipant.permissions?.canPublish ?? credentials.canPublish;
  update({ phase: "connected", canPublish });
  // Only where the server lets this person talk: sent off stage meanwhile, they come back muted.
  if (options.microphone && canPublish) await setMicrophone(true);
}

/** Fresh credentials for what the person holds now, from the venue they are inside. */
async function freshCredentials(
  queryClient: QueryClient,
  account: Account,
  venue: Pick<ActiveVenue, "kind" | "id">,
): Promise<spaces.AudioCredentials | undefined> {
  if (venue.kind === "space") {
    const response = await spaceService(account).joinSpace({ spaceId: venue.id });
    if (!isActive(account)) return undefined;
    rememberSpace(queryClient, response.space);
    if (response.me) update({ role: response.me.role });
    return response.credentials;
  }
  const response = await hangoutService(account).joinHangout({ hangoutId: venue.id });
  if (!isActive(account)) return undefined;
  rememberHangout(queryClient, response.hangout);
  return response.credentials;
}

let client: QueryClient | null = null;

const pause = (attempt: number) =>
  new Promise((resolve) => setTimeout(resolve, Math.min(1000 * 2 ** attempt, 15_000)));

/**
 * LiveKit gave the connection up. The server decides what is true: someone
 * who was removed, banned or left behind by a room that ended lands on that
 * state and is not reconnected. Anyone else enters again, a few times.
 */
async function disconnected(mine: number, reason: DisconnectReason | undefined): Promise<void> {
  const active = useAudioSession.getState().active;
  const account = owner;
  if (!active || !client || !account) return;
  const venue = { kind: active.kind, id: active.id };
  if (reason === DisconnectReason.DUPLICATE_IDENTITY)
    return void close({ ...venue, reason: "elsewhere" });
  const cut =
    reason === DisconnectReason.PARTICIPANT_REMOVED || reason === DisconnectReason.ROOM_DELETED;
  const microphone = active.micOn;
  update({ phase: "reconnecting", micOn: false });
  for (let attempt = 0; attempt < MAX_REJOINS; attempt++) {
    if (attempt > 0) await pause(attempt);
    if (mine !== generation || !isActive(account)) return;
    try {
      const exit = await whyOutside(client, account, venue, cut);
      if (mine !== generation) return;
      if (exit) return void close({ ...venue, reason: exit });
      const credentials = await freshCredentials(client, account, venue);
      if (mine !== generation) return;
      await connect(credentials, mine, { microphone });
      return;
    } catch (error) {
      if (mine !== generation) return;
      // Entering again was refused for good: the venue is gone for this person.
      if (isNotFound(error) || errorReason(error) === "BANNED_FROM_SPACE")
        return void close({ ...venue, reason: goneReason(venue.kind) });
      // The network, the API or LiveKit is not there yet: wait and try again.
    }
  }
  if (mine === generation) void close({ ...venue, reason: "lost" });
}

/** Asks the server whether the person is still inside; the reason they are not, otherwise. */
async function whyOutside(
  queryClient: QueryClient,
  account: Account,
  venue: Pick<ActiveVenue, "kind" | "id">,
  cut: boolean,
): Promise<ExitReason | null> {
  try {
    if (venue.kind === "space") {
      const { space } = await spaceService(account).getSpace({ spaceId: venue.id });
      rememberSpace(queryClient, space);
      return whyOutsideSpace(space, cut);
    }
    const { hangout } = await hangoutService(account).getHangout({ hangoutId: venue.id });
    rememberHangout(queryClient, hangout);
    return whyOutsideHangout(hangout);
  } catch (error) {
    if (isNotFound(error)) return goneReason(venue.kind);
    throw error;
  }
}

/** Enters a live room as whatever the person is there (host, co-host, listener). */
export async function enterSpace(
  queryClient: QueryClient,
  space: Pick<spaces.Space, "id" | "title">,
): Promise<boolean> {
  const account = currentAccount();
  if (!account) return false;
  client = queryClient;
  await close();
  const mine = begin(
    account,
    { kind: "space", id: space.id, title: space.title, emoji: "" },
    spaces.SpaceRole.LISTENER,
  );
  const service = spaceService(account);
  try {
    const response = await service.joinSpace({ spaceId: space.id });
    if (mine !== generation || !isActive(account)) return false;
    rememberSpace(queryClient, response.space);
    update({
      title: response.space?.title ?? space.title,
      role: response.me?.role ?? response.space?.viewer?.role ?? spaces.SpaceRole.LISTENER,
    });
    await connect(response.credentials, mine);
    return mine === generation;
  } catch (error) {
    if (mine !== generation) return false;
    await close();
    if (!isActive(account)) return false;
    fail(error);
    void queryClient.invalidateQueries({ queryKey: spacesKeys.space(space.id) });
    // Entered on the server but never connected: do not stay inside as a ghost.
    void service.leaveSpace({ spaceId: space.id }).catch(() => {});
    return false;
  }
}

/**
 * Puts the person on air as a host: `call` starts their upcoming room or
 * creates one on the spot, and the session connects with the credentials it
 * answers with. Resolves to the room, or `null` when it did not happen.
 */
async function goLive(
  queryClient: QueryClient,
  call: (
    service: ReturnType<typeof spaceService>,
  ) => Promise<{ space?: spaces.Space; credentials?: spaces.AudioCredentials }>,
): Promise<spaces.Space | null> {
  const account = currentAccount();
  if (!account) return null;
  client = queryClient;
  let response: Awaited<ReturnType<typeof call>>;
  try {
    response = await call(spaceService(account));
  } catch (error) {
    if (isActive(account)) fail(error);
    return null;
  }
  const space = response.space;
  if (!isActive(account) || !space) return null;
  await close();
  const mine = begin(
    account,
    { kind: "space", id: space.id, title: space.title, emoji: "" },
    space.viewer?.role ?? spaces.SpaceRole.HOST,
  );
  rememberSpace(queryClient, space);
  try {
    await connect(response.credentials, mine);
  } catch {
    if (mine !== generation) return space;
    // The room is live and the person's: they can come back in from its screen.
    Alert.alert(t("toast.connectFailed"));
    await close();
  }
  return space;
}

/** Starts the person's own upcoming room and connects to it as its host. */
export function startSpace(queryClient: QueryClient, spaceId: string) {
  return goLive(queryClient, (service) => service.startSpace({ spaceId }));
}

/** Opens a room right now, with the person as its host. */
export function createSpaceNow(
  queryClient: QueryClient,
  title: string,
  audience: spaces.SpaceAudience,
) {
  return goLive(queryClient, (service) => service.createSpace({ title, audience }));
}

/** Enters a hangout's voice room; everyone inside can talk, so the microphone opens. */
export async function enterHangout(
  queryClient: QueryClient,
  hangout: Pick<spaces.Hangout, "id" | "name" | "emoji">,
): Promise<boolean> {
  const account = currentAccount();
  if (!account) return false;
  client = queryClient;
  await close();
  const mine = begin(
    account,
    { kind: "hangout", id: hangout.id, title: hangout.name, emoji: hangout.emoji },
    spaces.SpaceRole.SPEAKER,
  );
  const service = hangoutService(account);
  try {
    const response = await service.joinHangout({ hangoutId: hangout.id });
    if (mine !== generation || !isActive(account)) return false;
    rememberHangout(queryClient, response.hangout);
    await connect(response.credentials, mine, { microphone: true });
    return mine === generation;
  } catch (error) {
    if (mine !== generation) return false;
    await close();
    if (!isActive(account)) return false;
    fail(error);
    void queryClient.invalidateQueries({ queryKey: spacesKeys.hangout(hangout.id) });
    void service.leaveHangout({ hangoutId: hangout.id }).catch(() => {});
    return false;
  }
}

/** Leaves whatever the person is inside, quietly. */
export async function leaveVenue(queryClient: QueryClient): Promise<void> {
  const active = useAudioSession.getState().active;
  const account = owner;
  if (!active || !account) return;
  await close();
  try {
    if (active.kind === "space") {
      markLeftSpace(queryClient, active.id);
      await spaceService(account).leaveSpace({ spaceId: active.id });
      if (isActive(account))
        void queryClient.invalidateQueries({ queryKey: spacesKeys.space(active.id) });
    } else {
      await hangoutService(account).leaveHangout({ hangoutId: active.id });
      if (isActive(account)) markLeftHangout(queryClient, active.id);
    }
  } catch {
    // Gone already (the room ended, the hangout closed): there is nothing left to leave.
  }
}

/**
 * `account` leaves the device (`lifecycle.ts`): the connection closes and
 * the server hears that the person left, with that account's own API.
 */
export async function leaveAs(account: Account): Promise<void> {
  const active = useAudioSession.getState().active;
  if (!active || owner?.api !== account.api) return;
  await close();
  try {
    if (active.kind === "space") await spaceService(account).leaveSpace({ spaceId: active.id });
    else await hangoutService(account).leaveHangout({ hangoutId: active.id });
  } catch {
    // The server drops a connection that is gone after its grace anyway.
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
  update({ role });
  // Back in the audience: the microphone goes at once, whatever LiveKit has heard so far.
  if (!stage) void setMicrophone(false);
  const mine = generation;
  promoteTimer = setTimeout(() => {
    if (mine !== generation || !room) return;
    if (Boolean(room.localParticipant.permissions?.canPublish) !== stage)
      void reconnectWithFreshToken(queryClient);
  }, 4000);
}

async function reconnectWithFreshToken(queryClient: QueryClient): Promise<boolean> {
  const active = useAudioSession.getState().active;
  const account = owner;
  if (!active || !account) return false;
  const mine = generation;
  try {
    const credentials = await freshCredentials(queryClient, account, active);
    if (mine !== generation) return false;
    await connect(credentials, mine);
    return mine === generation;
  } catch (error) {
    if (mine === generation && isActive(account)) fail(error);
    return false;
  }
}

async function setMicrophone(on: boolean): Promise<void> {
  const session = room;
  if (!session) return;
  try {
    if (on && !(await microphoneAllowed())) throw new Error("microphone refused");
    await session.localParticipant.setMicrophoneEnabled(on);
    if (room === session) update({ micOn: session.localParticipant.isMicrophoneEnabled });
  } catch {
    if (room !== session) return;
    update({ micOn: false });
    if (on) Alert.alert(i18n.t("mobile:spaces.noMicrophone"));
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

/**
 * The voices of the venue the person is inside (`kind` and `id` name it);
 * nothing while they are outside, where there is no connection to read from.
 */
export function useVoices(kind: ActiveVenue["kind"], id: string): Voices {
  const active = useAudioSession((state) =>
    state.active?.kind === kind && state.active.id === id ? state.active : null,
  );
  if (!active || active.phase === "connecting") return SILENCE;
  // The revision changes when the connection does: handing it over keeps the read fresh.
  return readVoices(active.revision);
}

function readVoices(_revision: number): Voices {
  if (!room) return SILENCE;
  return voicesOf(
    [room.localParticipant, ...room.remoteParticipants.values()],
    lastSpoke,
    Date.now(),
  );
}

/** Whether the person is inside this very venue with this phone. */
export function useIsInside(kind: ActiveVenue["kind"], id: string): boolean {
  return useAudioSession((state) => state.active?.kind === kind && state.active.id === id);
}
