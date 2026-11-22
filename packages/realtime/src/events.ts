/**
 * Frames as listeners see them.
 *
 * On the wire a domain's event travels inside `tokuly.realtime.v1.Event`: the
 * full name of its message plus the encoded bytes, so that the gateway's
 * protocol does not depend on the domains. Here it becomes what listeners
 * work with: a frame whose `body` is a discriminated union with one case per
 * domain, holding the decoded event.
 *
 *     client.onFrame((frame) => {
 *       if (frame.body.case === "wallet") applyWalletEvent(frame.body.value); // a WalletEvent
 *     });
 */
import { fromBinary, type MessageShape } from "@bufbuild/protobuf";
import {
  ask,
  chat,
  community,
  emoji,
  engagement,
  geo,
  live,
  media,
  notes,
  notify,
  plans,
  quest,
  realtime,
  school,
  social,
  spaces,
  video,
  wallet,
} from "@tokuly/api";

/**
 * The events the gateway carries, by the `case` listeners see them under.
 * A domain that starts pushing events gets a line here (and one in the list
 * in `realtime.proto`).
 */
const EVENTS = {
  geo: geo.GeoEventSchema,
  social: social.SocialEventSchema,
  chat: chat.ChatEventSchema,
  notify: notify.NotifyEventSchema,
  live: live.LiveEventSchema,
  video: video.VideoEventSchema,
  notes: notes.NotesEventSchema,
  media: media.MediaEventSchema,
  wallet: wallet.WalletEventSchema,
  engagement: engagement.EngagementEventSchema,
  quest: quest.QuestEventSchema,
  plans: plans.PlansEventSchema,
  school: school.SchoolEventSchema,
  spaces: spaces.SpacesEventSchema,
  community: community.CommunityEventSchema,
  ask: ask.AskEventSchema,
  emoji: emoji.EmojiEventSchema,
} as const;

type EventCase = keyof typeof EVENTS;

/** A domain's event: `{ case: "wallet", value: WalletEvent }` and so on. */
type EventBody = {
  [Case in EventCase]: { case: Case; value: MessageShape<(typeof EVENTS)[Case]> };
}[EventCase];

/** The gateway's own messages (welcome, pong, presence...), or nothing. */
type GatewayBody = Exclude<realtime.ServerFrame["body"], { case: "event" }>;

export type ServerFrameBody = GatewayBody | EventBody;

/** A frame from the server, with a domain's event decoded into `body`. */
export interface ServerFrame {
  /** The `ClientFrame.id` this frame answers; 0 for pushed events. */
  replyTo: bigint;
  /** Topic for topic-scoped events; empty for user-scoped ones. */
  topic: string;
  body: ServerFrameBody;
}

/** Message full name (`tokuly.wallet.v1.WalletEvent`) → the case it is delivered as. */
const CASE_BY_TYPE = new Map<string, EventCase>(
  (Object.keys(EVENTS) as EventCase[]).map((eventCase) => [EVENTS[eventCase].typeName, eventCase]),
);

/** The full names of the event messages this client understands. */
export const KNOWN_EVENT_TYPES: readonly string[] = [...CASE_BY_TYPE.keys()];

function decodeEvent(event: realtime.Event): EventBody | null {
  const eventCase = CASE_BY_TYPE.get(event.type);
  if (!eventCase) return null;
  try {
    // The case and the schema come from the same table entry; the compiler cannot see that.
    return { case: eventCase, value: fromBinary(EVENTS[eventCase], event.payload) } as EventBody;
  } catch {
    return null;
  }
}

/**
 * Reads one frame off the wire.
 *
 * Returns `null` for what a client has to skip: bytes that are not a frame,
 * and events it cannot read — above all events of a type it does not know,
 * which a server newer than this client may send at any time.
 */
export function decodeServerFrame(bytes: Uint8Array): ServerFrame | null {
  let frame: realtime.ServerFrame;
  try {
    frame = fromBinary(realtime.ServerFrameSchema, bytes);
  } catch {
    return null;
  }
  const { replyTo, topic, body } = frame;
  if (body.case !== "event") return { replyTo, topic, body };
  const event = decodeEvent(body.value);
  return event && { replyTo, topic, body: event };
}
