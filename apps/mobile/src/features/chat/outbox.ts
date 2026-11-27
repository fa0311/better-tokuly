import type { QueryClient } from "@tanstack/react-query";
import { chat, type common, create, fromDate, media } from "@tokuly/api";
import type { PendingState } from "@tokuly/core/chat";
import { readConversation, storeCreated, storeUpdated } from "@tokuly/core/chat";
import { randomUUID } from "expo-crypto";
import { create as createStore } from "zustand";
import { errorMessage } from "#/lib/errors";
import { realtime } from "#/lib/realtime";
import { type LocalFile, uploadAndProcess } from "#/lib/upload";
import { chatSync, fetchConversation, messageClient } from "./data";

/** A photo waiting to go out with a message. */
export interface OutgoingPhoto extends LocalFile {
  width?: number;
  height?: number;
  /** Set once the bytes are on the server, so a retry does not send them again. */
  assetId?: string;
}

/**
 * A message on its way to the server. It is drawn in the conversation the
 * moment it is sent and lives here until the server has it (or refused it:
 * then it waits with "retry" until the person decides).
 */
export interface Outgoing {
  /** The idempotency key: a retry with it can never send twice. */
  clientId: string;
  conversationId: string;
  createdAt: number;
  text: string;
  photos: OutgoingPhoto[];
  /** A stamp goes alone: no text, no photos. */
  stamp?: common.StampRef;
  replyTo?: chat.Message;
  status: "sending" | "failed";
  progress: number;
  error?: string;
}

export const useOutbox = createStore<{ items: Record<string, Outgoing[]> }>(() => ({ items: {} }));

const NONE: Outgoing[] = [];

export function useOutgoing(conversationId: string): Outgoing[] {
  return useOutbox((state) => state.items[conversationId] ?? NONE);
}

function find(conversationId: string, clientId: string): Outgoing | undefined {
  return useOutbox.getState().items[conversationId]?.find((item) => item.clientId === clientId);
}

function edit(conversationId: string, change: (items: Outgoing[]) => Outgoing[]): void {
  useOutbox.setState((state) => ({
    items: { ...state.items, [conversationId]: change(state.items[conversationId] ?? []) },
  }));
}

function patch(conversationId: string, clientId: string, change: Partial<Outgoing>): void {
  edit(conversationId, (items) =>
    items.map((item) => (item.clientId === clientId ? { ...item, ...change } : item)),
  );
}

/** Drops an outgoing message once the stored one is known (the event can beat the response). */
export function settle(conversationId: string, clientId: string): void {
  edit(conversationId, (items) => items.filter((item) => item.clientId !== clientId));
}

/** How a message being sent is drawn: as the message it will become. */
export function previewMessage(
  item: Outgoing,
  me: common.UserSummary,
): { message: chat.Message; state: PendingState } {
  const message = create(chat.MessageSchema, {
    id: `pending:${item.clientId}`,
    conversationId: item.conversationId,
    seq: BigInt(Number.MAX_SAFE_INTEGER),
    sender: me,
    kind: item.stamp
      ? chat.MessageKind.STAMP
      : item.photos.length > 0
        ? chat.MessageKind.MEDIA
        : chat.MessageKind.TEXT,
    clientId: item.clientId,
    text: item.text,
    stamp: item.stamp,
    attachments: item.photos.map((photo) => ({
      kind: media.AssetKind.IMAGE,
      image: { url: photo.uri, width: photo.width ?? 0, height: photo.height ?? 0 },
    })),
    replyTo: item.replyTo
      ? {
          messageId: item.replyTo.id,
          seq: item.replyTo.seq,
          sender: item.replyTo.sender,
          kind: item.replyTo.kind,
          text: item.replyTo.text.slice(0, 120),
        }
      : undefined,
    createdAt: fromDate(new Date(item.createdAt)),
  });
  return {
    message,
    state: {
      status: item.status,
      progress: item.photos.length > 0 ? item.progress : undefined,
      error: item.error,
    },
  };
}

/** A conversation's messages go out one after the other, in the order they were written. */
const queues = new Map<string, Promise<void>>();

function inOrder(conversationId: string, work: () => Promise<void>): Promise<void> {
  const next = (queues.get(conversationId) ?? Promise.resolve()).then(work, work);
  queues.set(conversationId, next);
  return next;
}

async function upload(item: Outgoing): Promise<string[]> {
  const loaded = new Array<number>(item.photos.length).fill(0);
  const report = () =>
    patch(item.conversationId, item.clientId, {
      progress: loaded.reduce((sum, part) => sum + part, 0) / Math.max(1, loaded.length),
    });
  return Promise.all(
    item.photos.map(async (photo, index) => {
      if (photo.assetId) return photo.assetId;
      const asset = await uploadAndProcess(photo, {
        purpose: media.AssetPurpose.CHAT_IMAGE,
        onProgress: (fraction) => {
          loaded[index] = fraction;
          report();
        },
      });
      const current = find(item.conversationId, item.clientId);
      if (current)
        patch(item.conversationId, item.clientId, {
          photos: current.photos.map((held, at) =>
            at === index ? { ...held, assetId: asset.id } : held,
          ),
        });
      return asset.id;
    }),
  );
}

function deliver(
  queryClient: QueryClient,
  conversationId: string,
  clientId: string,
): Promise<void> {
  return inOrder(conversationId, async () => {
    const item = find(conversationId, clientId);
    if (!item) return; // Discarded while it waited.
    try {
      const assetIds = await upload(item);
      if (!find(conversationId, clientId)) return; // Discarded while its photos went up.
      const { message } = await messageClient().sendMessage({
        conversationId,
        clientId,
        text: item.text,
        attachmentAssetIds: assetIds,
        stampId: item.stamp?.id ?? "",
        replyToMessageId: item.replyTo?.id ?? "",
      });
      if (!message) return;
      const viewer = readConversation(queryClient, conversationId)?.viewer;
      const stored = storeCreated(queryClient, message);
      if (stored.missed) void chatSync.pager.catchUp(queryClient, conversationId).catch(() => {});
      storeUpdated(queryClient, message, false);
      settle(conversationId, clientId);
      // Writing answers a request, and a request of one's own counts down: the viewer's state changed.
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
        error: errorMessage(error, { namespace: "chat" }),
      });
    }
  });
}

/** Sends a message: it shows up at once and goes to the server in the background. */
export function send(
  queryClient: QueryClient,
  conversationId: string,
  draft: {
    text: string;
    photos?: OutgoingPhoto[];
    stamp?: common.StampRef;
    replyTo?: chat.Message;
  },
): void {
  const item: Outgoing = {
    clientId: randomUUID(),
    conversationId,
    createdAt: Date.now(),
    text: draft.text,
    photos: draft.photos ?? [],
    stamp: draft.stamp,
    replyTo: draft.replyTo,
    status: "sending",
    progress: 0,
  };
  edit(conversationId, (items) => [...items, item]);
  realtime().sendTyping(conversationId, false);
  void deliver(queryClient, conversationId, item.clientId);
}

/** Tries a failed message again, with the same idempotency key. */
export function retry(queryClient: QueryClient, item: Outgoing): void {
  patch(item.conversationId, item.clientId, { status: "sending", error: undefined });
  void deliver(queryClient, item.conversationId, item.clientId);
}

export function discard(item: Outgoing): void {
  settle(item.conversationId, item.clientId);
}

/** Forgets everything on its way: the account left, and none of it may be sent, or seen, by the next one. */
export function clearOutbox(): void {
  useOutbox.setState({ items: {} });
  queues.clear();
}
