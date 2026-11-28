import {
  type InfiniteData,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { create, engagement, toDate } from "@tokuly/api";
import { Plus, X } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, ListRow, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { commentKeys, moderationClient } from "./api";
import { AuthorAvatar, authorHandle, authorName } from "./author";

const MAX_WORDS = 200;
const MAX_WORD_LENGTH = 50;

/** The owner's screening rules: hold everything, hold links, and the words that send a comment to review. */
export function Rules() {
  const { t } = useTranslation("comments");
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState("");
  const rules = useQuery({
    queryKey: commentKeys.rules,
    queryFn: async () =>
      (await moderationClient().getModerationSettings({})).settings ??
      create(engagement.ModerationSettingsSchema),
  });

  const save = useMutation({
    mutationFn: (settings: engagement.ModerationSettings) =>
      moderationClient().updateModerationSettings({ settings }),
    // The switch answers at once; if the server refuses, what it has comes back.
    onMutate: (settings) => {
      const previous = queryClient.getQueryData<engagement.ModerationSettings>(commentKeys.rules);
      queryClient.setQueryData(commentKeys.rules, settings);
      return { previous };
    },
    onSuccess: ({ settings }) => {
      if (settings) queryClient.setQueryData(commentKeys.rules, settings);
    },
    onError: (error, _settings, context) => {
      queryClient.setQueryData(commentKeys.rules, context?.previous);
      toast.error(errorMessage(error, { namespace: "comments" }));
    },
  });

  if (rules.isPending) return <Skeleton className="h-64 rounded-card" />;
  if (rules.isError || !rules.data)
    return <EmptyState emoji="😵‍💫" title={errorMessage(rules.error, { namespace: "comments" })} />;
  const settings = rules.data;
  const change = (patch: Partial<engagement.ModerationSettings>) =>
    save.mutate({ ...settings, ...patch });

  const addWords = () => {
    // Several words can be typed at once, separated by commas or new lines.
    const words = draft
      .split(/[,、\n]/)
      .map((word) => word.trim().slice(0, MAX_WORD_LENGTH))
      .filter(
        (word) =>
          word &&
          !settings.blockedWords.some((existing) => existing.toLowerCase() === word.toLowerCase()),
      );
    setDraft("");
    if (words.length === 0) return;
    if (settings.blockedWords.length + words.length > MAX_WORDS) {
      toast.error(t("rules.tooMany", { count: MAX_WORDS }));
      return;
    }
    change({ blockedWords: [...settings.blockedWords, ...new Set(words)] });
  };

  return (
    <div className="flex flex-col gap-6">
      <section>
        <h2 className="px-1 text-lg font-black">{t("rules.reviewTitle")}</h2>
        <p className="mt-0.5 px-1 text-sm leading-relaxed text-ink-soft">{t("rules.reviewLead")}</p>
        <div className="sticker mt-3 rounded-card bg-surface p-2">
          <SwitchRow
            label={t("rules.holdAll")}
            hint={t("rules.holdAllHint")}
            checked={settings.holdAll}
            onChange={(holdAll) => change({ holdAll })}
          />
          <SwitchRow
            label={t("rules.holdLinks")}
            hint={t("rules.holdLinksHint")}
            checked={settings.holdLinks}
            onChange={(holdLinks) => change({ holdLinks })}
          />
        </div>
      </section>

      <section>
        <h2 className="px-1 text-lg font-black">{t("rules.wordsTitle")}</h2>
        <p className="mt-0.5 px-1 text-sm leading-relaxed text-ink-soft">{t("rules.wordsLead")}</p>
        <div className="sticker mt-3 rounded-card bg-surface p-4">
          <form
            className="flex gap-2"
            onSubmit={(event) => {
              event.preventDefault();
              addWords();
            }}
          >
            <input
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              placeholder={t("rules.wordPlaceholder")}
              aria-label={t("rules.wordPlaceholder")}
              className="h-11 min-w-0 flex-1 rounded-control border-2 border-line bg-surface px-4 text-[15px] outline-none placeholder:text-ink-faint focus:border-brand"
            />
            <Button
              type="submit"
              variant="secondary"
              icon={<Plus className="size-4" />}
              disabled={!draft.trim()}
            >
              {t("rules.addWord")}
            </Button>
          </form>
          {settings.blockedWords.length === 0 ? (
            <p className="mt-3 text-sm text-ink-faint">{t("rules.noWords")}</p>
          ) : (
            <ul className="mt-3 flex flex-wrap gap-2" aria-label={t("rules.wordsTitle")}>
              {settings.blockedWords.map((word) => (
                <li
                  key={word}
                  className="inline-flex h-8 items-center gap-1 rounded-full bg-surface-2 pr-1 pl-3 text-sm font-bold"
                >
                  {word}
                  <button
                    type="button"
                    aria-label={t("rules.removeWord", { word })}
                    className="grid size-6 place-items-center rounded-full hover:bg-surface-3"
                    onClick={() =>
                      change({
                        blockedWords: settings.blockedWords.filter((existing) => existing !== word),
                      })
                    }
                  >
                    <X className="size-3.5" />
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      </section>
    </div>
  );
}

/** People the owner stopped from commenting, each with "unblock". */
export function BlockedCommenters() {
  const { t, i18n } = useTranslation("comments");
  const queryClient = useQueryClient();
  const list = useInfiniteQuery({
    queryKey: commentKeys.blocked,
    queryFn: ({ pageParam }) =>
      moderationClient().listBlockedCommenters({ page: { limit: 30, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const unblock = useMutation({
    mutationFn: (blocked: engagement.BlockedCommenter) =>
      moderationClient().unblockCommenter({ blockId: blocked.id }),
    onSuccess: (_response, blocked) => {
      queryClient.setQueryData<InfiniteData<engagement.ListBlockedCommentersResponse>>(
        commentKeys.blocked,
        (data) =>
          data
            ? {
                ...data,
                pages: data.pages.map((page) => ({
                  ...page,
                  commenters: page.commenters.filter((entry) => entry.id !== blocked.id),
                })),
              }
            : data,
      );
      toast.success(t("toast.unblocked", { name: authorName(blocked.commenter) }));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "comments" })),
  });
  const commenters = list.data?.pages.flatMap((page) => page.commenters) ?? [];

  return (
    <div className="flex flex-col gap-3">
      <p className="px-1 text-sm leading-relaxed text-ink-soft">{t("blocked.lead")}</p>
      {list.isPending ? (
        <Skeleton className="h-16" />
      ) : list.isError ? (
        <EmptyState emoji="😵‍💫" title={errorMessage(list.error, { namespace: "comments" })} />
      ) : commenters.length === 0 ? (
        <EmptyState emoji="🕊️" title={t("blocked.empty")} body={t("blocked.emptyBody")} />
      ) : (
        <div className="sticker rounded-card bg-surface p-2">
          {commenters.map((blocked) => (
            <ListRow
              key={blocked.id}
              leading={<AuthorAvatar author={blocked.commenter} size="md" />}
              title={authorName(blocked.commenter) || t("item.unknown")}
              subtitle={`@${authorHandle(blocked.commenter)} · ${toDate(blocked.createdAt)?.toLocaleDateString(i18n.language) ?? ""}`}
              trailing={
                <Button
                  size="sm"
                  variant="secondary"
                  loading={unblock.isPending && unblock.variables?.id === blocked.id}
                  onClick={() => unblock.mutate(blocked)}
                >
                  {t("blocked.unblock")}
                </Button>
              }
            />
          ))}
          <LoadMore query={list} />
        </div>
      )}
    </div>
  );
}
