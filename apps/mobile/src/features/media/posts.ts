import { ConnectError, common, media, notes, shorts } from "@tokuly/api";
import { randomUUID } from "expo-crypto";
import { create } from "zustand";
import { errorMessage } from "#/lib/errors";
import { forgetFiles, keepFile, keptPosts, readQueue, writeQueue } from "#/platform/transfer";
import { type Account, AccountChanged, currentAccount, ensureActive, isActive } from "./account";
import { type UploadState, uploadFile } from "./upload";

/** A photo or video picked from the library or taken with the camera. */
export interface LocalMedia {
  uri: string;
  kind: "image" | "video";
  name: string;
  contentType: string;
  /** Videos: seconds, when the picker or the recorder knew it. */
  duration?: number;
}

export interface StoryDraft {
  title: string;
  captions: string[];
  audience: notes.StoryAudience;
  /** CLOSE_FRIENDS: who may see it. */
  audienceUserIds: string[];
}

export interface ShortDraft {
  caption: string;
  visibility: common.Visibility;
  /** VISIBILITY_CIRCLE: which of the creator's circles. */
  circleId: string;
  commentsEnabled: boolean;
}

export type PostStatus =
  /** Bytes are going up, or the server is processing them. */
  | "uploading"
  /** Everything is uploaded; the story or short is being created. */
  | "posting"
  /** Its account is not the one in use: it goes on when that account is back. */
  | "paused"
  /** Interrupted (no network, the app was closed): it goes on by itself at the next chance. */
  | "interrupted"
  /** The server refused it (`error` says why, in its words); retried only by hand. */
  | "failed"
  | "done";

interface PostMedia {
  kind: LocalMedia["kind"];
  upload: UploadState;
  /** Set once the asset is ready to attach, so a retry does not upload it again. */
  assetId?: string;
}

/**
 * A story or a short on its way: its files are uploaded, then it is created.
 * It belongs to the account it was started as (`sessionId`) and is never
 * attached as anyone else: a switch pauses it, it goes on once that account
 * is back, and it is thrown away when that account leaves the device. It is
 * kept on disk so that it survives the app going to the background or being
 * closed.
 */
export interface Post {
  /** Also the idempotency key of the story. */
  id: string;
  sessionId: string;
  kind: "story" | "short";
  story?: StoryDraft;
  short?: ShortDraft;
  media: PostMedia[];
  status: PostStatus;
  /** 0..1 over all of its files. */
  progress: number;
  /** Why it failed, ready to show. */
  error?: string;
  /** The story or short it became. */
  resultId?: string;
  createdAt: number;
}

export const usePosts = create<{ posts: Post[] }>(() => ({ posts: [] }));

/** The posts of the account in use, newest first (another account's never show). */
export function usePostsOf(sessionId: string | undefined): Post[] {
  return usePosts((state) => state.posts).filter((post) => post.sessionId === sessionId);
}

function find(id: string): Post | undefined {
  return usePosts.getState().posts.find((post) => post.id === id);
}

function edit(id: string, change: (post: Post) => Post): void {
  usePosts.setState((state) => ({
    posts: state.posts.map((post) => (post.id === id ? change(post) : post)),
  }));
  writeQueue(usePosts.getState().posts);
}

function patch(id: string, change: Partial<Post>): void {
  edit(id, (post) => ({ ...post, ...change }));
}

const PURPOSE = { story: media.AssetPurpose.STORY, short: media.AssetPurpose.SHORT } as const;

/** The work running now, by post: what a switch stops before the account leaves. */
const running = new Map<string, { controller: AbortController; done: Promise<void> }>();

/** Errors that will not go away by sending again: the server's answer, shown as it gave it. */
function isRefusal(error: unknown): boolean {
  if (error instanceof ConnectError) return Boolean(error.metadata.get("x-tokuly-error"));
  return error instanceof Error && error.name === "AssetFailedError";
}

/**
 * Creates the story or the short. `signal` is the post's: a switch aborts the
 * request (an idempotency key, or the upload a short is made from, keeps a
 * retry from posting twice).
 */
async function publish(
  account: Account,
  post: Post,
  assetIds: string[],
  signal: AbortSignal,
): Promise<string> {
  if (post.kind === "story") {
    const draft = post.story!;
    const { story } = await account.api.service(notes.StoryService).createStory(
      {
        title: draft.title,
        items: assetIds.map((assetId, index) => ({
          assetId,
          caption: draft.captions[index] ?? "",
        })),
        audience: draft.audience,
        audienceUserIds:
          draft.audience === notes.StoryAudience.CLOSE_FRIENDS ? draft.audienceUserIds : [],
        idempotencyKey: post.id,
      },
      { signal },
    );
    return story?.id ?? "";
  }
  const draft = post.short!;
  try {
    const { short } = await account.api.service(shorts.ShortService).createShort(
      {
        assetId: assetIds[0],
        caption: draft.caption,
        visibility: draft.visibility,
        circleId: draft.visibility === common.Visibility.CIRCLE ? draft.circleId : "",
        commentsEnabled: draft.commentsEnabled,
      },
      { signal },
    );
    return short?.id ?? "";
  } catch (error) {
    // A short is made once per upload: an answer lost on the way means it exists already.
    if (
      error instanceof ConnectError &&
      error.metadata.get("x-tokuly-error") === "ASSET_ALREADY_USED"
    )
      return "";
    throw error;
  }
}

async function work(account: Account, id: string, signal: AbortSignal): Promise<void> {
  const post = find(id)!;
  patch(id, { status: "uploading", error: undefined });
  const share = 1 / post.media.length;
  const assetIds: string[] = [];
  for (const [index, item] of post.media.entries()) {
    if (item.assetId) {
      assetIds.push(item.assetId);
      continue;
    }
    const asset = await uploadFile(account, PURPOSE[post.kind], item.upload, {
      // A short goes live by itself once its upload is processed; a story needs it ready.
      until: post.kind === "short" ? "stored" : "ready",
      signal,
      save: (upload) =>
        edit(id, (current) => ({
          ...current,
          media: current.media.map((held, at) => (at === index ? { ...held, upload } : held)),
        })),
      onProgress: (fraction) => patch(id, { progress: (index + fraction) * share }),
    });
    edit(id, (current) => ({
      ...current,
      media: current.media.map((held, at) =>
        at === index ? { ...held, assetId: asset.id } : held,
      ),
    }));
    assetIds.push(asset.id);
  }
  ensureActive(account);
  patch(id, { status: "posting", progress: 1 });
  const resultId = await publish(account, find(id)!, assetIds, signal);
  patch(id, { status: "done", resultId });
  forgetFiles(id);
}

/**
 * Runs a post as its own account, if that is the one in use (otherwise it
 * waits). Uploads go on from where they stopped.
 */
export function runPost(id: string): Promise<void> {
  const existing = running.get(id);
  if (existing) return existing.done;
  const post = find(id);
  const account = currentAccount();
  if (!post || post.status === "done" || !account || account.sessionId !== post.sessionId)
    return Promise.resolve();
  const controller = new AbortController();
  const done = work(account, id, controller.signal)
    .catch((error: unknown) => {
      if (!find(id)) return; // Given up meanwhile.
      // Stopped for a switch (or the switch came first): it waits for its account.
      if (error instanceof AccountChanged || controller.signal.aborted || !isActive(account))
        return patch(id, { status: "paused" });
      const message = errorMessage(error, {
        namespace: post.kind === "story" ? "stories" : "shorts",
      });
      if (isRefusal(error)) patch(id, { status: "failed", error: message });
      else patch(id, { status: "interrupted", error: message });
    })
    .finally(() => running.delete(id));
  running.set(id, { controller, done });
  return done;
}

/** Starts a story or a short as the account in use: its files are kept on the device first. */
export async function submitPost(
  draft: { kind: "story"; story: StoryDraft } | { kind: "short"; short: ShortDraft },
  files: LocalMedia[],
): Promise<string> {
  const account = currentAccount();
  if (!account) throw new AccountChanged();
  const id = randomUUID();
  const kept = await Promise.all(
    files.map(async (file, index) => {
      const copy = await keepFile(id, file.uri, `${index}-${file.name}`);
      return {
        kind: file.kind,
        upload: { uri: copy.uri, name: file.name, contentType: file.contentType, size: copy.size },
      };
    }),
  );
  // Picking and copying took a moment: the post is only the account's if it is still the one in use.
  if (!isActive(account)) {
    forgetFiles(id);
    throw new AccountChanged();
  }
  const post: Post = {
    id,
    sessionId: account.sessionId,
    ...draft,
    media: kept,
    status: "uploading",
    progress: 0,
    createdAt: Date.now(),
  };
  usePosts.setState((state) => ({ posts: [post, ...state.posts] }));
  writeQueue(usePosts.getState().posts);
  void runPost(id);
  return id;
}

/** Tries a failed or interrupted post again, from where it stopped. */
export function retryPost(id: string): void {
  void runPost(id);
}

/** Gives a post up: its work stops and its files are thrown away. */
export function discardPost(id: string): void {
  running.get(id)?.controller.abort();
  usePosts.setState((state) => ({ posts: state.posts.filter((post) => post.id !== id) }));
  writeQueue(usePosts.getState().posts);
  forgetFiles(id);
}

/** A finished post leaves the list once it was seen. */
export function dismissPost(id: string): void {
  usePosts.setState((state) => ({ posts: state.posts.filter((post) => post.id !== id) }));
  writeQueue(usePosts.getState().posts);
}

/**
 * Before the account in use leaves (a switch, a sign-out): everything it has
 * running stops, and is waited for, so that nothing it started can make a
 * request once another account's tokens are in place.
 */
export async function pauseRunning(): Promise<void> {
  const all = [...running.values()];
  for (const { controller } of all) controller.abort();
  await Promise.all(all.map(({ done }) => done));
}

/**
 * Picks the queue up: on start, and whenever the app comes back to the
 * front. The active account's posts go on; those of accounts no longer on
 * this device (signed out) are thrown away with their files — the server
 * deletes their unused uploads by itself.
 */
export async function resumePosts(onDevice: ReadonlySet<string>): Promise<void> {
  if (usePosts.getState().posts.length === 0) usePosts.setState({ posts: await readQueue<Post>() });
  const { posts } = usePosts.getState();
  const kept = posts.filter((post) => onDevice.has(post.sessionId) && post.status !== "done");
  if (kept.length !== posts.length) {
    usePosts.setState({ posts: kept });
    writeQueue(kept);
  }
  const ids = new Set(kept.map((post) => post.id));
  for (const orphan of keptPosts()) if (!ids.has(orphan)) forgetFiles(orphan);
  const account = currentAccount();
  for (const post of kept)
    if (post.sessionId === account?.sessionId && post.status !== "failed") void runPost(post.id);
}
