import type { QueryClient } from "@tanstack/react-query";
import { chat, type common, create, fromDate } from "@tokuly/api";
import type { PendingState } from "@tokuly/core/chat";
import { create as createStore } from "zustand";
import { realtime } from "#/lib/realtime";
import { uploadAndProcess, uploadFile } from "#/lib/upload";
import { messageClient, povClient } from "./api";
import { localAttachment, type OutgoingFile, PURPOSE, releaseFiles } from "./attachments";
import { fetchConversation, readConversation } from "./conversations";
import { errorMessage } from "./errors";
import { storeCreated, storeUpdated } from "./messages";

/**
 * Messages on their way to the server. A message appears in the conversation
 * the moment it is sent; this is where it lives until the server has it (or
 * refuses it, when it stays with a "retry" until the person decides).
 */
export interface Outgoing {
  /** The idempotency key: retries with it can never send twice. */
  clientId: string;
  conversationId: string;
  createdAt: number;
  text: string;
  stamp?: common.StampRef;
  files: OutgoingFile[];
  location?: { lat: number; lng: number; name: string; address: string };
  /** The message this one answers. */
  replyTo?: chat.Message;
  /** Set when this is the answer to a POV request (sent through PovService). */
  povRequestId?: string;
  status: "sending" | "failed";
  /** Upload progress, 0..1. */
  progress: number;
  error?: string;
}

interface OutboxStore {
  /** conversation → messages being sent, oldest first. */
  items: Record<string, Outgoing[]>;
}

export const useOutbox = createStore<OutboxStore>(() => ({ items: {} }));

const NONE: Outgoing[] = [];

/** The messages of a conversation still on their way. */
export function useOutgoing(conversationId: string): Outgoing[] {
  return useOutbox((state) => state.items[conversationId] ?? NONE);
}

function find(conversationId: string, clientId: string): Outgoing | undefined {
  return useOutbox.getState().items[conversationId]?.find((item) => item.clientId === clientId);
}

function patch(conversationId: string, clientId: string, change: Partial<Outgoing>): void {
  useOutbox.setState((state) => ({
    items: {
      ...state.items,
      [conversationId]: (state.items[conversationId] ?? []).map((item) =>
        item.clientId === clientId ? { ...item, ...change } : item,
      ),
    },
  }));
}

function remove(conversationId: string, clientId: string): void {
  useOutbox.setState((state) => ({
    items: {
      ...state.items,
      [conversationId]: (state.items[conversationId] ?? []).filter(
        (item) => item.clientId !== clientId,
      ),
    },
  }));
}

/** Drops an outgoing message once the stored one is known (the event can beat the response). */
export function settle(conversationId: string, clientId: string): void {
  const item = find(conversationId, clientId);
  if (!item) return;
  remove(conversationId, clientId);
  // The stored message's pictures replace the local previews; give them a moment to load first.
  setTimeout(() => releaseFiles(item.files), 30_000);
}

/** How a message being sent is drawn: as the message it will become. */
export function previewMessage(
  item: Outgoing,
  me: common.UserSummary,
): { message: chat.Message; state: PendingState } {
  const kind = item.stamp
    ? chat.MessageKind.STAMP
    : item.files.length > 0
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
    attachments: item.files.map(localAttachment),
    location: item.location
      ? {
          position: { lat: item.location.lat, lng: item.location.lng },
          name: item.location.name,
          address: item.location.address,
        }
      : undefined,
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
      progress: item.files.length > 0 ? item.progress : undefined,
      error: item.error,
    },
  };
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
const queues = new Map<string, Promise<void>>();

function inOrder(conversationId: string, work: () => Promise<void>): Promise<void> {
  const next = (queues.get(conversationId) ?? Promise.resolve()).then(work, work);
  queues.set(conversationId, next);
  return next;
}

async function upload(item: Outgoing): Promise<string[]> {
  const total = item.files.reduce((sum, file) => sum + file.file.size, 0) || 1;
  const loaded = new Map<string, number>();
  const report = () =>
    patch(item.conversationId, item.clientId, {
      progress: Math.min(1, [...loaded.values()].reduce((sum, bytes) => sum + bytes, 0) / total),
    });
  return Promise.all(
    item.files.map(async (file) => {
      if (file.assetId) {
        loaded.set(file.id, file.file.size);
        return file.assetId;
      }
      const options = {
        purpose: PURPOSE[file.kind],
        onProgress: (fraction: number) => {
          loaded.set(file.id, fraction * file.file.size);
          report();
        },
      };
      // A recording is a nameless blob; the upload wants a file name.
      const named =
        file.file instanceof File
          ? file.file
          : new File([file.file], file.name, { type: file.file.type });
      // A video may be sent while it is still being transcoded; everything else must be ready.
      const asset =
        file.kind === "video"
          ? await uploadFile(named, options)
          : await uploadAndProcess(named, options);
      // Remember the upload, so a retry does not send the bytes again.
      const current = find(item.conversationId, item.clientId);
      if (current)
        patch(item.conversationId, item.clientId, {
          files: current.files.map((held) =>
            held.id === file.id ? { ...held, assetId: asset.id } : held,
          ),
        });
      return asset.id;
    }),
  );
}

/**
 * Takes one message to the server: its files first, then the message. The
 * whole of it waits its turn in the conversation's queue, so what was written
 * first arrives first, even when it carries a photo and the next one is text.
 */
function deliver(
  queryClient: QueryClient,
  conversationId: string,
  clientId: string,
): Promise<void> {
  return inOrder(conversationId, async () => {
    const item = find(conversationId, clientId);
    // Discarded while it was waiting.
    if (!item) return;
    try {
      const assetIds = await upload(item);
      // Discarded while its files were uploading.
      if (!find(conversationId, clientId)) return;
      const message = item.povRequestId
        ? (
            await povClient().answerPov({
              requestId: item.povRequestId,
              attachmentAssetIds: assetIds,
              clientId,
              text: item.text,
            })
          ).message
        : (
            await messageClient().sendMessage({
              conversationId,
              clientId,
              text: item.text,
              stampId: item.stamp?.id ?? "",
              attachmentAssetIds: assetIds,
              location: item.location
                ? {
                    position: { lat: item.location.lat, lng: item.location.lng },
                    name: item.location.name,
                    address: item.location.address,
                  }
                : undefined,
              replyToMessageId: item.replyTo?.id ?? "",
            })
          ).message;
      if (!message) return;
      const before = readConversation(queryClient, conversationId);
      storeCreated(queryClient, message);
      storeUpdated(queryClient, message, false);
      settle(conversationId, clientId);
      // Writing in a message request answers it, and a request of one's own counts down: both change the viewer's state.
      const viewer = before?.viewer;
      if (
        !viewer ||
        viewer.requestState !== chat.RequestState.ACCEPTED ||
        viewer.requestMessagesLeft >= 0
      )
        void fetchConversation(queryClient, conversationId).catch(() => {});
    } catch (error) {
      // A message that failed waits for the person; the ones behind it go on.
      patch(conversationId, clientId, {
        status: "failed",
        error: errorMessage(error),
      });
    }
  });
}

export type Draft = Pick<Outgoing, "text"> &
  Partial<Pick<Outgoing, "stamp" | "files" | "location" | "replyTo" | "povRequestId">>;

/** Sends a message: it shows up in the conversation at once and goes to the server in the background. */
export function send(queryClient: QueryClient, conversationId: string, draft: Draft): void {
  const item: Outgoing = {
    clientId: crypto.randomUUID(),
    conversationId,
    createdAt: Date.now(),
    text: draft.text,
    stamp: draft.stamp,
    files: draft.files ?? [],
    location: draft.location,
    replyTo: draft.replyTo,
    povRequestId: draft.povRequestId,
    status: "sending",
    progress: 0,
  };
  useOutbox.setState((state) => ({
    items: { ...state.items, [conversationId]: [...(state.items[conversationId] ?? []), item] },
  }));
  realtime().sendTyping(conversationId, false);
  void deliver(queryClient, conversationId, item.clientId);
}

/** Tries a failed message again (with the same idempotency key). */
export function retry(queryClient: QueryClient, item: Outgoing): void {
  patch(item.conversationId, item.clientId, { status: "sending", error: undefined });
  void deliver(queryClient, item.conversationId, item.clientId);
}

/** Forgets everything on its way (the account signed out: none of it may be sent, or seen, by the next one). */
export function clearOutbox(): void {
  for (const items of Object.values(useOutbox.getState().items))
    for (const item of items) releaseFiles(item.files);
  useOutbox.setState({ items: {} });
  queues.clear();
}

/** Gives up on a message that is not on the server. */
export function discard(item: Outgoing): void {
  remove(item.conversationId, item.clientId);
  releaseFiles(item.files);
}
