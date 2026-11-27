import { chat, type common, create, fromDate, type media } from "@tokuly/api";
import type { PendingState } from "./timeline";

/**
 * Messages on their way to the server, as far as every app handles them the
 * same: how one is drawn until the server has it, the order they go out in,
 * and whether sending changed the conversation. What they carry (files,
 * photos) and how it is uploaded stay in each app.
 */

/** A place sent as a message. */
export interface OutgoingPlace {
  lat: number;
  lng: number;
  name: string;
  address: string;
}

/** A message on its way, without what it carries. */
export interface OutgoingMessage {
  /** The idempotency key: retries with it can never send twice. */
  clientId: string;
  conversationId: string;
  createdAt: number;
  text: string;
  stamp?: common.StampRef;
  location?: OutgoingPlace;
  /** The message this one answers. */
  replyTo?: chat.Message;
  /** Set when this is the answer to a POV request (sent through PovService). */
  povRequestId?: string;
  status: "sending" | "failed";
  /** Upload progress, 0..1. */
  progress: number;
  error?: string;
}

/**
 * How a message being sent is drawn: as the message it will become, with
 * `attachments` standing for what it carries (local previews).
 */
export function previewMessage(
  item: OutgoingMessage,
  attachments: media.Attachment[],
  me: common.UserSummary,
): { message: chat.Message; state: PendingState } {
  const kind = item.stamp
    ? chat.MessageKind.STAMP
    : attachments.length > 0
      ? chat.MessageKind.MEDIA
      : item.location
        ? chat.MessageKind.LOCATION
        : chat.MessageKind.TEXT;
  const answered = item.replyTo;
  const message = create(chat.MessageSchema, {
    id: `pending:${item.clientId}`,
    conversationId: item.conversationId,
    seq: BigInt(Number.MAX_SAFE_INTEGER),
    sender: me,
    kind,
    clientId: item.clientId,
    text: item.text,
    bigEmojiCount: kind === chat.MessageKind.TEXT ? bigEmojiCount(item.text) : 0,
    stamp: item.stamp,
    attachments,
    location: item.location ? placeOf(item.location) : undefined,
    replyTo: answered
      ? {
          messageId: answered.id,
          seq: answered.seq,
          sender: answered.sender,
          kind: answered.kind,
          text: answered.text.slice(0, 120),
          thumbnailUrl:
            answered.stamp?.imageUrl ||
            answered.attachments[0]?.image?.url ||
            answered.attachments[0]?.video?.poster?.url ||
            "",
        }
      : undefined,
    povReply: item.povRequestId ? { requestId: item.povRequestId } : undefined,
    createdAt: fromDate(new Date(item.createdAt)),
  });
  return {
    message,
    state: {
      status: item.status,
      progress: attachments.length > 0 ? item.progress : undefined,
      error: item.error,
    },
  };
}

/** A place as the API carries it in a message. */
export function placeOf(place: OutgoingPlace): chat.LocationShare {
  return create(chat.LocationShareSchema, {
    position: { lat: place.lat, lng: place.lng },
    name: place.name,
    address: place.address,
  });
}

const segmenter =
  typeof Intl !== "undefined" && "Segmenter" in Intl
    ? new Intl.Segmenter(undefined, { granularity: "grapheme" })
    : undefined;
const EMOJI = /^(\p{Extended_Pictographic}|\p{Regional_Indicator}{2}|[#*0-9]️?⃣)/u;

/** 1-3 when the text is nothing but that many emoji (drawn large), like the server counts them. */
export function bigEmojiCount(text: string): number {
  const trimmed = text.trim();
  if (!trimmed || trimmed.length > 64 || !segmenter) return 0;
  let count = 0;
  for (const { segment } of segmenter.segment(trimmed)) {
    if (/^\s+$/.test(segment)) continue;
    if (!EMOJI.test(segment)) return 0;
    if (++count > 3) return 0;
  }
  return count;
}

/** One conversation's deliveries run one after the other, in the order they were written. */
export class SendQueue {
  private readonly tails = new Map<string, Promise<void>>();

  /** Runs `work` once everything queued before it in the conversation is done (or failed). */
  run(conversationId: string, work: () => Promise<void>): Promise<void> {
    const next = (this.tails.get(conversationId) ?? Promise.resolve()).then(work, work);
    this.tails.set(conversationId, next);
    return next;
  }

  clear(): void {
    this.tails.clear();
  }
}

/**
 * Whether sending a message changed the viewer's side of the conversation
 * (`viewer` as it was before): writing in a message request answers it, and
 * a request of one's own counts down. The conversation is read again then.
 */
export function sendChangesViewer(viewer: chat.ViewerState | undefined): boolean {
  return (
    !viewer || viewer.requestState !== chat.RequestState.ACCEPTED || viewer.requestMessagesLeft >= 0
  );
}
