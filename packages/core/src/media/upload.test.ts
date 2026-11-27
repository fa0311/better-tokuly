import { create, type MessageInitShape, media } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { AssetFailedError } from "../errors";
import { processed, uploadBytes } from "./upload";

/** Bytes as a string stands for them: its length is the size. */
class Text {
  constructor(readonly text: string) {}
  get size() {
    return this.text.length;
  }
  slice(start: number, end: number) {
    return new Text(this.text.slice(start, end));
  }
}

const asset = (status: media.AssetStatus, error = "") =>
  create(media.AssetSchema, { id: "a1", status, error });

/** A media service that answers with `created` and records what it was asked. */
function service(
  created: MessageInitShape<typeof media.CreateUploadResponseSchema>,
  watched: media.Asset[] = [],
) {
  const calls: string[] = [];
  const client = {
    createUpload: async () => {
      calls.push("create");
      return create(media.CreateUploadResponseSchema, { uploadId: "u1", ...created });
    },
    getUploadParts: async ({ partNumbers }: { partNumbers: number[] }) => ({
      parts: partNumbers.map((part) => ({ partNumber: part, putUrl: `part-${part}` })),
    }),
    completeUpload: async () => {
      calls.push("complete");
      return { asset: asset(media.AssetStatus.PROCESSING) };
    },
    abortUpload: async () => {
      calls.push("abort");
      return {};
    },
    async *watchAsset() {
      for (const update of watched) yield { asset: update };
    },
  };
  return { calls, client: () => client as never };
}

const request = { purpose: media.AssetPurpose.CHAT_IMAGE, fileName: "a.jpg", contentType: "" };

describe("uploads", () => {
  it("send a small file in one request", async () => {
    const { calls, client } = service({ putUrl: "single", putHeaders: { "x-a": "1" } });
    const sent: string[] = [];
    const progress: number[] = [];
    await uploadBytes(
      client,
      request,
      new Text("abcd"),
      async (url, body, headers, onProgress) => {
        sent.push(`${url}:${body.text}:${headers["x-a"]}`);
        onProgress(2);
      },
      { onProgress: (fraction) => progress.push(fraction) },
    );
    expect(sent).toEqual(["single:abcd:1"]);
    expect(progress).toEqual([0.5, 1]);
    expect(calls).toEqual(["create", "complete"]);
  });

  it("send a large file in parts", async () => {
    const { client } = service({ partSizeBytes: 3n, partCount: 3 });
    const sent: string[] = [];
    await uploadBytes(client, request, new Text("abcdefgh"), async (url, body) => {
      sent.push(`${url}:${body.text}`);
    });
    expect(sent.sort()).toEqual(["part-1:abc", "part-2:def", "part-3:gh"]);
  });

  it("are aborted when the bytes do not get through", async () => {
    const { calls, client } = service({ putUrl: "single" });
    await expect(
      uploadBytes(client, request, new Text("ab"), () => Promise.reject(new Error("offline"))),
    ).rejects.toThrow("offline");
    expect(calls).toEqual(["create", "abort"]);
  });
});

describe("processing", () => {
  it("waits until the asset is ready", async () => {
    const { client } = service({}, [
      asset(media.AssetStatus.PROCESSING),
      asset(media.AssetStatus.READY),
    ]);
    const ready = await processed(client, asset(media.AssetStatus.PROCESSING));
    expect(ready.status).toBe(media.AssetStatus.READY);
  });

  it("says why the server refused the asset", async () => {
    const { client } = service({}, [asset(media.AssetStatus.FAILED, "MEDIA_TOO_LONG")]);
    const refused = processed(client, asset(media.AssetStatus.PROCESSING));
    await expect(refused).rejects.toBeInstanceOf(AssetFailedError);
    await expect(refused).rejects.toMatchObject({ reason: "MEDIA_TOO_LONG" });
  });
});
