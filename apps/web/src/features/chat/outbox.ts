import type { QueryClient } from "@tanstack/react-query";
import type { chat, common } from "@tokuly/api";
import {
  type OutgoingMessage,
  type PendingState,
  placeOf,
  previewMessage as preview,
  SendQueue,
  sendChangesViewer,
} from "@tokuly/core/chat";
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
export interface Outgoing extends OutgoingMessage {
  files: OutgoingFile[];
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
  return preview(item, item.files.map(localAttachment), me);
}

const queue = new SendQueue();

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
  return queue.run(conversationId, async () => {
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
              location: item.location ? placeOf(item.location) : undefined,
              replyToMessageId: item.replyTo?.id ?? "",
            })
          ).message;
      if (!message) return;
      const before = readConversation(queryClient, conversationId);
      storeCreated(queryClient, message);
      storeUpdated(queryClient, message, false);
      settle(conversationId, clientId);
      if (sendChangesViewer(before?.viewer))
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
  queue.clear();
}

/** Gives up on a message that is not on the server. */
export function discard(item: Outgoing): void {
  remove(item.conversationId, item.clientId);
  releaseFiles(item.files);
}
