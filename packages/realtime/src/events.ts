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
 *
 * A domain's schemas are loaded with the first event of that domain, not at
 * startup: together they are most of the generated code. Until they are there,
 * that frame and every frame after it wait in a {@link ServerFrameReader}, so
 * listeners still see them in the order they came.
 */
import { type DescMessage, fromBinary, type MessageShape } from "@bufbuild/protobuf";
import * as realtime from "@tokuly/api/gen/tokuly/realtime/v1/realtime_pb.js";

/**
 * The events the gateway carries, by the `case` listeners see them under:
 * the full name of the message and where its schema is. A domain that starts
 * pushing events gets a line here (and one in the list in `realtime.proto`).
 * The imports are plain strings, so every bundler (Metro too) can follow them.
 */
const EVENTS = {
  geo: {
    type: "tokuly.geo.v1.GeoEvent",
    schema: () => import("@tokuly/api/gen/tokuly/geo/v1/geo_pb.js").then((f) => f.GeoEventSchema),
  },
  social: {
    type: "tokuly.social.v1.SocialEvent",
    schema: () =>
      import("@tokuly/api/gen/tokuly/social/v1/social_pb.js").then((f) => f.SocialEventSchema),
  },
  chat: {
    type: "tokuly.chat.v1.ChatEvent",
    schema: () =>
      import("@tokuly/api/gen/tokuly/chat/v1/chat_pb.js").then((f) => f.ChatEventSchema),
  },
  notify: {
    type: "tokuly.notify.v1.NotifyEvent",
    schema: () =>
      import("@tokuly/api/gen/tokuly/notify/v1/notify_pb.js").then((f) => f.NotifyEventSchema),
  },
  live: {
    type: "tokuly.live.v1.LiveEvent",
    schema: () =>
      import("@tokuly/api/gen/tokuly/live/v1/live_pb.js").then((f) => f.LiveEventSchema),
  },
  video: {
    type: "tokuly.video.v1.VideoEvent",
    schema: () =>
      import("@tokuly/api/gen/tokuly/video/v1/video_pb.js").then((f) => f.VideoEventSchema),
  },
  notes: {
    type: "tokuly.notes.v1.NotesEvent",
    schema: () =>
      import("@tokuly/api/gen/tokuly/notes/v1/notes_pb.js").then((f) => f.NotesEventSchema),
  },
  media: {
    type: "tokuly.media.v1.MediaEvent",
    schema: () =>
      import("@tokuly/api/gen/tokuly/media/v1/media_pb.js").then((f) => f.MediaEventSchema),
  },
  wallet: {
    type: "tokuly.wallet.v1.WalletEvent",
    schema: () =>
      import("@tokuly/api/gen/tokuly/wallet/v1/wallet_pb.js").then((f) => f.WalletEventSchema),
  },
  engagement: {
    type: "tokuly.engagement.v1.EngagementEvent",
    schema: () =>
      import("@tokuly/api/gen/tokuly/engagement/v1/engagement_pb.js").then(
        (f) => f.EngagementEventSchema,
      ),
  },
  quest: {
    type: "tokuly.quest.v1.QuestEvent",
    schema: () =>
      import("@tokuly/api/gen/tokuly/quest/v1/quest_pb.js").then((f) => f.QuestEventSchema),
  },
  plans: {
    type: "tokuly.plans.v1.PlansEvent",
    schema: () =>
      import("@tokuly/api/gen/tokuly/plans/v1/plans_pb.js").then((f) => f.PlansEventSchema),
  },
  school: {
    type: "tokuly.school.v1.SchoolEvent",
    schema: () =>
      import("@tokuly/api/gen/tokuly/school/v1/school_pb.js").then((f) => f.SchoolEventSchema),
  },
  spaces: {
    type: "tokuly.spaces.v1.SpacesEvent",
    schema: () =>
      import("@tokuly/api/gen/tokuly/spaces/v1/spaces_pb.js").then((f) => f.SpacesEventSchema),
  },
  community: {
    type: "tokuly.community.v1.CommunityEvent",
    schema: () =>
      import("@tokuly/api/gen/tokuly/community/v1/community_pb.js").then(
        (f) => f.CommunityEventSchema,
      ),
  },
  ask: {
    type: "tokuly.ask.v1.AskEvent",
    schema: () => import("@tokuly/api/gen/tokuly/ask/v1/ask_pb.js").then((f) => f.AskEventSchema),
  },
  emoji: {
    type: "tokuly.emoji.v1.EmojiEvent",
    schema: () =>
      import("@tokuly/api/gen/tokuly/emoji/v1/emoji_pb.js").then((f) => f.EmojiEventSchema),
  },
  moments: {
    type: "tokuly.moments.v1.MomentsEvent",
    schema: () =>
      import("@tokuly/api/gen/tokuly/moments/v1/moments_pb.js").then((f) => f.MomentsEventSchema),
  },
} as const;

type EventCase = keyof typeof EVENTS;
type EventSchema<Case extends EventCase> = Awaited<ReturnType<(typeof EVENTS)[Case]["schema"]>>;

/** A domain's event: `{ case: "wallet", value: WalletEvent }` and so on. */
type EventBody = {
  [Case in EventCase]: { case: Case; value: MessageShape<EventSchema<Case>> };
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
  (Object.keys(EVENTS) as EventCase[]).map((eventCase) => [EVENTS[eventCase].type, eventCase]),
);

/** The full names of the event messages this client understands. */
export const KNOWN_EVENT_TYPES: readonly string[] = [...CASE_BY_TYPE.keys()];

/** Each domain's schema once it is loaded, or while it loads. A load that failed is tried again. */
const schemas = new Map<EventCase, DescMessage | Promise<DescMessage>>();

function schemaOf(eventCase: EventCase): DescMessage | Promise<DescMessage> {
  let schema = schemas.get(eventCase);
  if (!schema) {
    schema = EVENTS[eventCase].schema().then(
      (loaded) => {
        schemas.set(eventCase, loaded);
        return loaded;
      },
      (error: unknown) => {
        schemas.delete(eventCase);
        throw error;
      },
    );
    schemas.set(eventCase, schema);
  }
  return schema;
}

function decodeEvent(eventCase: EventCase, schema: DescMessage, payload: Uint8Array) {
  try {
    // The case and the schema come from the same table entry; the compiler cannot see that.
    return { case: eventCase, value: fromBinary(schema, payload) } as EventBody;
  } catch {
    return null;
  }
}

/** A frame, `null` for one to skip, or the promise of either while its domain's schema loads. */
type Read = ServerFrame | null | Promise<ServerFrame | null>;

function readServerFrame(bytes: Uint8Array): Read {
  let frame: realtime.ServerFrame;
  try {
    frame = fromBinary(realtime.ServerFrameSchema, bytes);
  } catch {
    return null;
  }
  const { replyTo, topic, body } = frame;
  if (body.case !== "event") return { replyTo, topic, body };
  const eventCase = CASE_BY_TYPE.get(body.value.type);
  if (!eventCase) return null;
  const { payload } = body.value;
  const withEvent = (event: EventBody | null) => event && { replyTo, topic, body: event };
  const schema = schemaOf(eventCase);
  if (!(schema instanceof Promise)) return withEvent(decodeEvent(eventCase, schema, payload));
  return schema.then(
    (loaded) => withEvent(decodeEvent(eventCase, loaded, payload)),
    // Offline, or the app was updated and the old chunk is gone: this one event is lost.
    () => null,
  );
}

/**
 * Reads one frame off the wire.
 *
 * Resolves to `null` for what a client has to skip: bytes that are not a
 * frame, and events it cannot read — above all events of a type it does not
 * know, which a server newer than this client may send at any time.
 */
export async function decodeServerFrame(bytes: Uint8Array): Promise<ServerFrame | null> {
  return readServerFrame(bytes);
}

/**
 * Reads the frames of one connection in the order they came. A frame is
 * handed on right away when it can be: the gateway's own messages, and events
 * of a domain whose schemas are loaded. The first event of a domain waits for
 * them, and every frame behind it waits too.
 */
export class ServerFrameReader {
  private readonly waiting: Read[] = [];

  constructor(private readonly deliver: (frame: ServerFrame) => void) {}

  read(bytes: Uint8Array): void {
    const frame = readServerFrame(bytes);
    if (this.waiting.length === 0 && !(frame instanceof Promise)) {
      if (frame) this.deliver(frame);
      return;
    }
    this.waiting.push(frame);
    if (this.waiting.length === 1) void this.drain();
  }

  private async drain(): Promise<void> {
    while (this.waiting.length > 0) {
      const frame = await this.waiting[0];
      this.waiting.shift();
      try {
        if (frame) this.deliver(frame);
      } catch (error) {
        // The frames behind it must not wait forever.
        console.error(error);
      }
    }
  }
}
