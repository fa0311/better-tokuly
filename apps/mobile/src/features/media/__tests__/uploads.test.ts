import { create, media, notes } from "@tokuly/api";
import { useSessionStore } from "#/features/session/store";
import { api } from "#/lib/api";
import { putFile, slicePart } from "#/platform/transfer";
import { pauseRunning, runPost, submitPost, usePosts } from "../posts";
import { uploadFile } from "../upload";

// The edges: the device's file transfer and the API object of the active account.
jest.mock("#/platform/transfer", () => ({
  keepFile: jest.fn(async (_post: string, uri: string) => ({ uri: `kept:${uri}`, size: 10 })),
  forgetFiles: jest.fn(),
  keptPosts: jest.fn(() => []),
  readQueue: jest.fn(async () => []),
  writeQueue: jest.fn(),
  slicePart: jest.fn(async (_uri: string, offset: number) => `part@${offset}`),
  deleteFile: jest.fn(),
  putFile: jest.fn(async () => {}),
}));
jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));

const READY = create(media.AssetSchema, { id: "asset-1", status: media.AssetStatus.READY });

/** A fake API object of one account: the media and story services, recording every call. */
function account(name: string) {
  const calls: string[] = [];
  const record =
    <T>(method: string, answer: (request: never) => T) =>
    async (request: never) => {
      calls.push(method);
      return answer(request);
    };
  const mediaService = {
    createUpload: record("createUpload", () => ({
      uploadId: `${name}-upload`,
      assetId: "asset-1",
      putUrl: "https://store/put",
      putHeaders: {},
      maxDurationSeconds: 60,
    })),
    getUploadParts: record("getUploadParts", (request: { partNumbers: number[] }) => ({
      completedPartNumbers: request.partNumbers.length === 0 ? [1] : [],
      parts: request.partNumbers.map((part) => ({
        partNumber: part,
        putUrl: `https://store/${part}`,
      })),
    })),
    completeUpload: record("completeUpload", () => ({ asset: READY })),
    getAsset: record("getAsset", () => ({ asset: READY })),
  };
  const storyService = {
    createStory: record("createStory", (request: { idempotencyKey: string }) => ({
      story: { id: `story-of-${request.idempotencyKey}` },
    })),
  };
  const object = {
    service: (service: { typeName: string }) =>
      service.typeName === media.MediaService.typeName ? mediaService : storyService,
  };
  return { object, calls, storyService };
}

function signIn(sessionId: string, apiObject: object) {
  jest.mocked(api).mockReturnValue(apiObject as never);
  useSessionStore.setState({
    state: { status: "signedIn", sessionId, me: { user: { id: `user-${sessionId}` } } as never },
  });
}

beforeEach(() => {
  jest.clearAllMocks();
  usePosts.setState({ posts: [] });
});

describe("uploads", () => {
  it("resume a multipart upload with the parts the store does not hold yet", async () => {
    const a = account("a");
    signIn("A", a.object);
    const asset = await uploadFile(
      { sessionId: "A", userId: "user-A", api: a.object as never },
      media.AssetPurpose.STORY,
      {
        uri: "file:///video.mp4",
        name: "video.mp4",
        contentType: "video/mp4",
        size: 10,
        uploadId: "a-upload",
        assetId: "asset-1",
        partSize: 4,
        partCount: 3,
      },
      {
        until: "ready",
        save: () => {},
        onProgress: () => {},
        signal: new AbortController().signal,
      },
    );
    expect(asset.id).toBe("asset-1");
    // Part 1 was in the store already: parts 2 (4 bytes from 4) and 3 (the last 2 bytes) go up.
    expect(jest.mocked(slicePart).mock.calls).toEqual([
      ["file:///video.mp4", 4, 4],
      ["file:///video.mp4", 8, 2],
    ]);
    expect(jest.mocked(putFile).mock.calls.map((call) => call[0])).toEqual([
      "https://store/2",
      "https://store/3",
    ]);
    expect(a.calls).not.toContain("createUpload");
    expect(a.calls.at(-1)).toBe("completeUpload");
  });

  it("never attach a story started as one account as another", async () => {
    const a = account("a");
    const b = account("b");
    signIn("A", a.object);
    // The bytes are on their way when another account takes over.
    let finishPut = () => {};
    jest.mocked(putFile).mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finishPut = resolve;
        }),
    );
    const id = await submitPost(
      {
        kind: "story",
        story: {
          title: "",
          captions: [""],
          audience: notes.StoryAudience.FRIENDS,
          audienceUserIds: [],
        },
      },
      [{ uri: "file:///photo.jpg", kind: "image", name: "photo.jpg", contentType: "image/jpeg" }],
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(putFile).toHaveBeenCalledTimes(1);
    signIn("B", b.object);
    finishPut();
    await runPost(id);

    expect(usePosts.getState().posts[0]?.status).toBe("paused");
    expect(a.calls).toEqual(["createUpload"]);
    expect(b.calls).toEqual([]);
    // As B, the post is not B's to run.
    await runPost(id);
    expect(b.calls).toEqual([]);

    // A is back (a new API object for the same session): the post goes on where it stopped, as A.
    const again = account("a");
    signIn("A", again.object);
    await runPost(id);
    expect(again.calls).toEqual(["completeUpload", "createStory"]);
    expect(b.calls).toEqual([]);
    expect(usePosts.getState().posts[0]).toMatchObject({
      status: "done",
      resultId: `story-of-${id}`,
    });
  });

  it("stop what is running before the account leaves", async () => {
    const a = account("a");
    signIn("A", a.object);
    jest.mocked(putFile).mockImplementationOnce(
      (_url, _uri, _headers, _progress, signal) =>
        new Promise<void>((_resolve, reject) => {
          signal.addEventListener("abort", () => reject(new Error("AbortError")));
        }),
    );
    await submitPost(
      {
        kind: "story",
        story: {
          title: "",
          captions: [""],
          audience: notes.StoryAudience.FRIENDS,
          audienceUserIds: [],
        },
      },
      [{ uri: "file:///photo.jpg", kind: "image", name: "photo.jpg", contentType: "image/jpeg" }],
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
    // What the feature's `beforeLeave` runs; the switch happens only once it is done.
    await pauseRunning();
    signIn("B", account("b").object);
    expect(usePosts.getState().posts[0]?.status).toBe("paused");
    expect(a.calls).toEqual(["createUpload"]);
  });

  it("abort creating the story when the account leaves, and create it once with the same key later", async () => {
    const a = account("a");
    signIn("A", a.object);
    // The story is being created when the switch comes: the request must not outlive the account.
    const keys: string[] = [];
    let aborted = false;
    a.storyService.createStory = (async (
      request: { idempotencyKey: string },
      options: { signal: AbortSignal },
    ) => {
      keys.push(request.idempotencyKey);
      await new Promise((_resolve, reject) =>
        options.signal.addEventListener("abort", () => {
          aborted = true;
          reject(new Error("aborted"));
        }),
      );
    }) as never;
    const id = await submitPost(
      {
        kind: "story",
        story: {
          title: "",
          captions: [""],
          audience: notes.StoryAudience.FRIENDS,
          audienceUserIds: [],
        },
      },
      [{ uri: "file:///photo.jpg", kind: "image", name: "photo.jpg", contentType: "image/jpeg" }],
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(usePosts.getState().posts[0]?.status).toBe("posting");
    await pauseRunning();
    expect(aborted).toBe(true);
    expect(usePosts.getState().posts[0]?.status).toBe("paused");

    // Back as A: created once, with the same idempotency key.
    const again = account("a");
    signIn("A", again.object);
    await runPost(id);
    expect(again.calls).toEqual(["createStory"]);
    expect(keys).toEqual([id]);
    expect(usePosts.getState().posts[0]).toMatchObject({
      status: "done",
      resultId: `story-of-${id}`,
    });
  });
});
