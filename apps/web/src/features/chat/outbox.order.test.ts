import { QueryClient } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The server and the object store, as far as sending needs them: uploads take a while, messages are instant.
const sent: string[] = [];
const uploads: { resolve(): void }[] = [];
vi.mock("./api", () => ({
  messageClient: () => ({
    sendMessage: async (request: { text: string; clientId: string; conversationId: string }) => {
      sent.push(request.text);
      if (request.text === "refused") throw new Error("refused");
      return { message: undefined };
    },
  }),
  povClient: () => ({}),
  conversationClient: () => ({}),
  chatKeys: {
    timeline: (id: string) => ["chat", "timeline", id],
    conversation: (id: string) => ["chat", "conversation", id],
    list: (filter: string) => ["chat", "list", filter],
    unread: ["chat", "unread"],
  },
  LIST_FILTERS: {},
}));
vi.mock("#/lib/realtime", () => ({ realtime: () => ({ sendTyping: () => {} }) }));
vi.mock("#/lib/upload", () => {
  const upload = () =>
    new Promise((resolve) =>
      uploads.push({ resolve: () => resolve({ id: `asset-${uploads.length}` }) }),
    );
  return { uploadFile: upload, uploadAndProcess: upload };
});

const { send, useOutbox, clearOutbox } = await import("./outbox");

const photo = () => ({
  id: "f1",
  file: new File(["x"], "a.png", { type: "image/png" }),
  name: "a.png",
  kind: "image" as const,
  previewUrl: "blob:a",
});
const settled = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
  sent.length = 0;
  uploads.length = 0;
  clearOutbox();
});

describe("sending", () => {
  it("delivers messages in the order they were written, even when an earlier one is still uploading", async () => {
    const queryClient = new QueryClient();
    send(queryClient, "c1", { text: "photo first", files: [photo()] });
    send(queryClient, "c1", { text: "text second" });
    await settled();
    // The text waits behind the photo.
    expect(sent).toEqual([]);
    uploads[0]?.resolve();
    await settled();
    await settled();
    expect(sent).toEqual(["photo first", "text second"]);
  });

  it("does not hold another conversation up", async () => {
    const queryClient = new QueryClient();
    send(queryClient, "c1", { text: "slow", files: [photo()] });
    send(queryClient, "c2", { text: "elsewhere" });
    await settled();
    expect(sent).toEqual(["elsewhere"]);
  });

  it("goes on with the next message after one failed, and keeps the failed one for a retry", async () => {
    const queryClient = new QueryClient();
    send(queryClient, "c1", { text: "refused" });
    send(queryClient, "c1", { text: "after" });
    await settled();
    await settled();
    expect(sent).toEqual(["refused", "after"]);
    expect(useOutbox.getState().items.c1?.find((item) => item.text === "refused")?.status).toBe(
      "failed",
    );
  });

  it("forgets everything on its way when the account signs out", async () => {
    const queryClient = new QueryClient();
    send(queryClient, "c1", { text: "waiting", files: [photo()] });
    clearOutbox();
    uploads[0]?.resolve();
    await settled();
    await settled();
    expect(sent).toEqual([]);
    expect(useOutbox.getState().items).toEqual({});
  });
});
