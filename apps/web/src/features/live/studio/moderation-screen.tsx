import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { errorReason, identity, toMillis } from "@tokuly/api";
import { Plus, UserMinus, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { Page } from "#/components/ui/page";
import { PaneSpinner } from "#/components/ui/spinner";
import { EmojiText } from "#/features/emoji";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { chatClient, liveKeys, nextCursor } from "../api";
import { formatMoment } from "../parts";
import { LiveChannelPicker, NoChannel, Section, useLiveChannel } from "./shared";

const fail = (error: unknown) => toast.error(errorMessage(error, { namespace: "live" }));

/** The people who help keep the channel's chats in order: appoint by handle, dismiss. */
function Moderators({ channelId }: { channelId: string }) {
  const { t } = useTranslation("live");
  const queryClient = useQueryClient();
  const [handle, setHandle] = useState("");
  const query = useQuery({
    queryKey: liveKeys.moderators(channelId),
    queryFn: async () => (await chatClient().listModerators({ channelId })).moderators,
  });
  const refresh = () => queryClient.invalidateQueries({ queryKey: liveKeys.moderators(channelId) });
  const add = useMutation({
    mutationFn: async () => {
      const wanted = handle.trim().replace(/^@/, "");
      let userId: string | undefined;
      try {
        const { profile } = await api()
          .service(identity.ProfileService)
          .getProfile({ by: { case: "handle", value: wanted } });
        userId = profile?.user?.id;
      } catch (error) {
        if (errorReason(error) !== "USER_NOT_FOUND") throw error;
      }
      if (!userId) throw new Error("no-user");
      return chatClient().addModerator({ channelId, userId });
    },
    onSuccess: ({ moderator }) => {
      setHandle("");
      toast.success(t("moderation.added", { name: moderator?.user?.displayName ?? "" }));
      void refresh();
    },
    onError: (error) =>
      error instanceof Error && error.message === "no-user"
        ? toast.error(t("moderation.noSuchUser"))
        : fail(error),
  });
  const remove = useMutation({
    mutationFn: (userId: string) => chatClient().removeModerator({ channelId, userId }),
    onSuccess: () => {
      toast.success(t("moderation.removed"));
      void refresh();
    },
    onError: fail,
  });
  return (
    <Section title={t("moderation.moderators")} lead={t("moderation.moderatorsLead")}>
      <form
        className="flex items-end gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          if (handle.trim()) add.mutate();
        }}
      >
        <TextField
          className="min-w-0 flex-1"
          label={t("moderation.handle")}
          placeholder="@handle"
          value={handle}
          autoCapitalize="none"
          onChange={(event) => setHandle(event.target.value)}
        />
        <Button
          type="submit"
          className="mb-0.5"
          loading={add.isPending}
          disabled={!handle.trim()}
          icon={<Plus className="size-4" />}
        >
          {t("moderation.add")}
        </Button>
      </form>
      {query.data?.length ? (
        <ul className="flex flex-col" data-testid="moderators">
          {query.data.map((moderator) => (
            <li key={moderator.user?.id} className="flex items-center gap-3 py-2">
              <Avatar user={moderator.user} size="sm" />
              <span className="min-w-0 flex-1">
                <span className="block truncate font-bold">
                  <EmojiText text={moderator.user?.displayName ?? ""} />
                </span>
                <span className="block truncate text-sm text-ink-soft">
                  @{moderator.user?.handle}
                </span>
              </span>
              <Button
                variant="ghost"
                size="sm"
                icon={<UserMinus className="size-4" />}
                onClick={() => remove.mutate(moderator.user?.id ?? "")}
              >
                {t("moderation.remove")}
              </Button>
            </li>
          ))}
        </ul>
      ) : (
        !query.isPending && (
          <p className="text-sm font-bold text-ink-faint">{t("moderation.noModerators")}</p>
        )
      )}
    </Section>
  );
}

/** Who is timed out or banned on the channel, and the way to let them back in. */
function Restrictions({ channelId }: { channelId: string }) {
  const { t, i18n } = useTranslation("live");
  const queryClient = useQueryClient();
  const query = useInfiniteQuery({
    queryKey: liveKeys.restrictions(channelId),
    queryFn: ({ pageParam }) =>
      chatClient().listChatRestrictions({ channelId, page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
  const lift = useMutation({
    mutationFn: (userId: string) => chatClient().unrestrictChatUser({ channelId, userId }),
    onSuccess: () => {
      toast.success(t("moderation.lifted"));
      void queryClient.invalidateQueries({ queryKey: liveKeys.restrictions(channelId) });
    },
    onError: fail,
  });
  const restrictions = query.data?.pages.flatMap((page) => page.restrictions) ?? [];
  return (
    <Section title={t("moderation.restrictions")} lead={t("moderation.restrictionsLead")}>
      {restrictions.length === 0 ? (
        !query.isPending && (
          <p className="text-sm font-bold text-ink-faint">{t("moderation.noRestrictions")}</p>
        )
      ) : (
        <ul className="flex flex-col" data-testid="restrictions">
          {restrictions.map((restriction) => (
            <li key={restriction.user?.id} className="flex items-center gap-3 py-2">
              <Avatar user={restriction.user} size="sm" />
              <span className="min-w-0 flex-1">
                <span className="block truncate font-bold">
                  <EmojiText text={restriction.user?.displayName ?? ""} />
                </span>
                <span className="block truncate text-sm text-ink-soft">
                  {restriction.expiresAt
                    ? t("moderation.until", {
                        time: formatMoment(toMillis(restriction.expiresAt), i18n.language),
                      })
                    : t("moderation.forGood")}
                  {restriction.reason && ` · ${restriction.reason}`}
                </span>
              </span>
              <Button
                variant="ghost"
                size="sm"
                loading={lift.isPending && lift.variables === restriction.user?.id}
                onClick={() => lift.mutate(restriction.user?.id ?? "")}
              >
                {t("moderation.lift")}
              </Button>
            </li>
          ))}
        </ul>
      )}
      <LoadMore query={query} />
    </Section>
  );
}

/** Words that may not appear in the channel's chats. */
function BannedWords({ channelId }: { channelId: string }) {
  const { t } = useTranslation("live");
  const queryClient = useQueryClient();
  const [word, setWord] = useState("");
  const query = useQuery({
    queryKey: liveKeys.bannedWords(channelId),
    queryFn: async () => (await chatClient().getBannedWords({ channelId })).words,
  });
  const words = query.data ?? [];
  const save = useMutation({
    mutationFn: (next: string[]) => chatClient().setBannedWords({ channelId, words: next }),
    onSuccess: (response) => {
      queryClient.setQueryData(liveKeys.bannedWords(channelId), response.words);
      setWord("");
    },
    onError: fail,
  });
  return (
    <Section title={t("moderation.words")} lead={t("moderation.wordsLead")}>
      <form
        className="flex items-end gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          const next = word.trim();
          if (next) save.mutate([...words, next]);
        }}
      >
        <TextField
          className="min-w-0 flex-1"
          label={t("moderation.word")}
          value={word}
          maxLength={30}
          onChange={(event) => setWord(event.target.value)}
        />
        <Button
          type="submit"
          className="mb-0.5"
          loading={save.isPending}
          disabled={!word.trim() || words.length >= 200}
          icon={<Plus className="size-4" />}
        >
          {t("moderation.addWord")}
        </Button>
      </form>
      {words.length > 0 ? (
        <ul className="flex flex-wrap gap-2" data-testid="banned-words">
          {words.map((existing) => (
            <li
              key={existing}
              className="flex h-9 items-center gap-1 rounded-full bg-surface-2 pr-1 pl-3 text-sm font-bold"
            >
              {existing}
              <button
                type="button"
                aria-label={t("moderation.removeWord", { word: existing })}
                onClick={() => save.mutate(words.filter((other) => other !== existing))}
                className="grid size-7 place-items-center rounded-full hover:bg-surface-3"
              >
                <X className="size-3.5" />
              </button>
            </li>
          ))}
        </ul>
      ) : (
        !query.isPending && (
          <p className="text-sm font-bold text-ink-faint">{t("moderation.noWords")}</p>
        )
      )}
    </Section>
  );
}

/** `/studio/live/moderation`: a channel's moderators, timeouts and bans, and banned words. */
export function ModerationScreen() {
  const { t } = useTranslation("live");
  const { query, channels, channel } = useLiveChannel();
  const channelId = channel?.summary?.id;
  return (
    <Page title={t("moderation.title")} back="/studio/live">
      {query.isPending ? (
        <PaneSpinner />
      ) : !channelId ? (
        <NoChannel />
      ) : (
        <div className="flex flex-col gap-4" key={channelId}>
          <LiveChannelPicker channels={channels} selectedId={channelId} />
          <Moderators channelId={channelId} />
          <Restrictions channelId={channelId} />
          <BannedWords channelId={channelId} />
        </div>
      )}
    </Page>
  );
}
