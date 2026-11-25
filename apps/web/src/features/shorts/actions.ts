import { type QueryKey, useMutation, useQueryClient } from "@tanstack/react-query";
import { engagement, shorts } from "@tokuly/api";
import { shortSubject } from "@tokuly/core/shorts";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { copyText, useEngagement, useReact } from "#/features/engagement";
import { errorMessage } from "#/lib/errors";
import {
  feedClient,
  patchShortEverywhere,
  patchViewer,
  removeShortEverywhere,
  shortClient,
  shortKeys,
} from "./api";

const { ShareTarget } = shorts;

/**
 * The like of a short. Likes belong to the engagement feature: its cache is
 * what every like button of the app reads, so a like made here shows there
 * and the other way round.
 */
export function useShortLike(short: shorts.Short) {
  const subject = shortSubject(short);
  const { data } = useEngagement(subject);
  const react = useReact(subject);
  const liked = data
    ? data.viewerReaction?.kind === engagement.ReactionKind.LIKE
    : (short.viewer?.liked ?? false);
  return {
    liked,
    count: data ? data.likeCount : Number(short.likeCount),
    comments: data ? data.commentCount : Number(short.commentCount),
    /** False while the short takes no likes (comments are off, or it is not published). */
    enabled: data?.interactionsEnabled ?? false,
    like: () => {
      if (!liked) void react({ kind: engagement.ReactionKind.LIKE, emoji: "" });
    },
    toggle: () => void react(liked ? undefined : { kind: engagement.ReactionKind.LIKE, emoji: "" }),
  };
}

/** Saves a short to the viewer's private list, or takes it out; shown at once. */
export function useSaveShort(short: shorts.Short) {
  const { t } = useTranslation("shorts");
  const queryClient = useQueryClient();
  const saved = short.viewer?.saved ?? false;
  const mutation = useMutation({
    mutationFn: async (save: boolean): Promise<bigint> =>
      (
        await (save
          ? shortClient().saveShort({ shortId: short.id })
          : shortClient().unsaveShort({ shortId: short.id }))
      ).saveCount,
    onMutate: (save) => {
      const before = { saved, saveCount: short.saveCount };
      patchViewer(
        queryClient,
        short.id,
        { saved: save },
        { saveCount: short.saveCount + (save ? 1n : short.saveCount > 0n ? -1n : 0n) },
      );
      return before;
    },
    onSuccess: (saveCount, save) => {
      patchViewer(queryClient, short.id, { saved: save }, { saveCount });
      void queryClient.invalidateQueries({ queryKey: ["shorts", "list", "saved"] });
      if (save) toast.success(t("save.done"));
    },
    onError: (error, _save, before) => {
      if (before)
        patchViewer(
          queryClient,
          short.id,
          { saved: before.saved },
          { saveCount: before.saveCount },
        );
      toast.error(errorMessage(error, { namespace: "shorts" }));
    },
  });
  return { saved, toggle: () => mutation.mutate(!saved) };
}

/**
 * Sharing a short: every way records the share (which is what the count
 * shows) and gets the link to pass on from the server.
 */
export function useShareShort(short: shorts.Short) {
  const { t } = useTranslation("shorts");
  const queryClient = useQueryClient();
  const record = async (target: shorts.ShareTarget): Promise<string> => {
    const response = await shortClient().shareShort({ shortId: short.id, target });
    patchShortEverywhere(queryClient, short.id, (current) => ({
      ...current,
      shareCount: response.shareCount,
    }));
    return response.url;
  };
  const failed = (error: unknown) => toast.error(errorMessage(error, { namespace: "shorts" }));
  return {
    canShareOut: typeof navigator !== "undefined" && typeof navigator.share === "function",
    copyLink: async () => {
      try {
        if (await copyText(await record(ShareTarget.COPY_LINK))) toast.success(t("share.copied"));
        else toast.error(t("share.copyFailed"));
      } catch (error) {
        failed(error);
      }
    },
    shareOut: async () => {
      try {
        const url = await record(ShareTarget.SYSTEM);
        await navigator.share({ url, title: short.caption || short.channel?.name });
      } catch (error) {
        // Closing the share sheet is not an error worth a toast.
        if (!(error instanceof DOMException && error.name === "AbortError")) failed(error);
      }
    },
    /** The chat sheet sends the link itself; this only counts the share. */
    recordChat: () => void record(ShareTarget.CHAT).catch(() => {}),
  };
}

/**
 * "Not interested": the short leaves every list at once, and the toast
 * offers to take it back, which puts the lists back as they were.
 */
export function useNotInterested(short: shorts.Short) {
  const { t } = useTranslation("shorts");
  const queryClient = useQueryClient();
  return async () => {
    const before: [QueryKey, unknown][] = queryClient.getQueriesData({ queryKey: shortKeys.all });
    // Marked on the short's own entry (a feed opened on it drops it), then gone from every list.
    patchViewer(queryClient, short.id, { notInterested: true });
    removeShortEverywhere(queryClient, short.id);
    const restore = () => {
      for (const [key, data] of before) queryClient.setQueryData(key, data);
    };
    try {
      await feedClient().markNotInterested({ shortId: short.id });
      toast.message(t("notInterested.done"), {
        action: {
          label: t("notInterested.undo"),
          onClick: () => {
            restore();
            feedClient()
              .undoNotInterested({ shortId: short.id })
              .catch((error: unknown) => {
                patchViewer(queryClient, short.id, { notInterested: true });
                removeShortEverywhere(queryClient, short.id);
                toast.error(errorMessage(error, { namespace: "shorts" }));
              });
          },
        },
      });
    } catch (error) {
      restore();
      toast.error(errorMessage(error, { namespace: "shorts" }));
    }
  };
}
