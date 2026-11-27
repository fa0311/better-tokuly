import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { notes, social, stamp } from "@tokuly/api";
import { friendKeys } from "@tokuly/core/friends";
import { Check, ChevronRight, Hash, Smile, UserPlus, Users } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Coins } from "#/components/ui/coins";
import { FreshNews } from "#/features/news";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { AppLink } from "./app-link";
import { formatCount } from "./format";
import { RecentPanel, TrendingPanel } from "./panels";

const noteClient = () => api().service(notes.NoteService);
const friendClient = () => api().service(social.FriendService);
const stampStoreClient = () => api().service(stamp.StampStoreService);

/**
 * What `/search` shows before a query: the person's recent searches, what is
 * trending, people they may know, fresh news and popular stamp packs. Every
 * block comes from an RPC that exists; one that has nothing to show (or whose
 * domain is not running) simply is not there.
 */
export function Explore() {
  return (
    <div className="grid grid-cols-1 gap-x-8 gap-y-7 lg:grid-cols-2">
      <div className="flex min-w-0 flex-col gap-7">
        <RecentPanel />
        <TrendingPanel limit={10} />
        <TrendingHashtags />
      </div>
      <div className="flex min-w-0 flex-col gap-7">
        <PeopleYouMayKnow />
        <FreshNews />
        <PopularPacks />
      </div>
    </div>
  );
}

function Block({
  title,
  icon,
  more,
  children,
}: {
  title: string;
  icon: ReactNode;
  more?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section aria-label={title}>
      <header className="mb-2 flex items-center gap-2 px-1">
        <h2 className="flex min-w-0 flex-1 items-center gap-2 text-lg font-black">
          {icon}
          <span className="truncate">{title}</span>
        </h2>
        {more}
      </header>
      {children}
    </section>
  );
}

/** Hashtags many people are using in notes right now. */
function TrendingHashtags() {
  const { t } = useTranslation("search");
  const trending = useQuery({
    queryKey: ["notes", "hashtags", "trending", 12],
    queryFn: () => noteClient().listTrendingHashtags({ limit: 12 }),
    staleTime: 60_000,
    retry: false,
  });
  const hashtags = (trending.data?.hashtags ?? []).flatMap((entry) =>
    entry.hashtag ? [entry.hashtag] : [],
  );
  if (hashtags.length === 0) return null;
  return (
    <Block title={t("explore.hashtags")} icon={<Hash className="size-5 shrink-0 text-ink-soft" />}>
      <ul className="flex flex-wrap gap-2">
        {hashtags.map((hashtag) => (
          <li key={hashtag.tag} className="max-w-full">
            <AppLink
              href={`/tag/${encodeURIComponent(hashtag.tag)}`}
              className="pressable sticker-sm inline-flex h-10 max-w-full items-center gap-1.5 rounded-full bg-surface px-3.5 text-sm font-bold"
            >
              <span className="grid size-5 shrink-0 place-items-center rounded-full bg-lime text-xs font-black text-[#17122b]">
                #
              </span>
              <span className="truncate">{hashtag.tag}</span>
              <span className="shrink-0 text-xs text-ink-faint tabular-nums">
                {formatCount(hashtag.noteCount)}
              </span>
            </AppLink>
          </li>
        ))}
      </ul>
    </Block>
  );
}

/** Friend suggestions from the social graph, with a one-tap request. */
function PeopleYouMayKnow() {
  const { t } = useTranslation("search");
  const suggestions = useQuery({
    queryKey: friendKeys.suggestionList(8),
    queryFn: () => friendClient().listSuggestions({ limit: 8 }),
    retry: false,
  });
  const people = (suggestions.data?.suggestions ?? []).filter(
    (suggestion) => suggestion.user && !suggestion.user.deleted,
  );
  if (people.length === 0) return null;
  return (
    <Block
      title={t("explore.people")}
      icon={<Users className="size-5 shrink-0 text-ink-soft" />}
      more={
        <Link
          to="/friends"
          className="inline-flex shrink-0 items-center text-sm font-bold text-brand hover:underline"
        >
          {t("explore.more")}
          <ChevronRight className="size-4" />
        </Link>
      }
    >
      {/* A row to swipe through on phones; a grid of the first six where a mouse cannot swipe. */}
      <ul className="scrollbar-none -mx-4 flex gap-3 overflow-x-auto px-4 pt-1 pb-2 md:mx-0 md:px-1 lg:grid lg:grid-cols-3 lg:overflow-visible lg:[&>li:nth-child(n+7)]:hidden">
        {people.map(
          (suggestion) =>
            suggestion.user && <PersonCard key={suggestion.user.id} suggestion={suggestion} />,
        )}
      </ul>
    </Block>
  );
}

function PersonCard({ suggestion }: { suggestion: social.Suggestion }) {
  const { t } = useTranslation("search");
  const queryClient = useQueryClient();
  const user = suggestion.user;
  const request = useMutation({
    mutationFn: (userId: string) =>
      friendClient().sendFriendRequest({
        target: { case: "userId", value: userId },
        source: social.FriendSource.SUGGESTION,
      }),
    onSuccess: () => {
      toast.success(t("explore.requested"));
      void queryClient.invalidateQueries({ queryKey: friendKeys.requests });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });
  if (!user) return null;
  return (
    <li className="sticker flex w-36 shrink-0 flex-col items-center gap-2 rounded-card bg-surface p-3 text-center lg:w-auto">
      <Link
        to="/u/$handle"
        params={{ handle: user.handle }}
        className="flex w-full min-w-0 flex-col items-center gap-1.5"
      >
        <Avatar user={user} size="lg" outlined />
        <span className="w-full truncate text-sm font-black">
          {user.displayName || user.handle}
        </span>
        <span className="w-full truncate text-xs text-ink-soft">
          {suggestion.mutualFriendCount > 0
            ? t("explore.mutual", { count: suggestion.mutualFriendCount })
            : `@${user.handle}`}
        </span>
      </Link>
      {request.isSuccess ? (
        <span className="inline-flex h-9 items-center gap-1 text-xs font-bold text-ink-soft">
          <Check className="size-4" />
          {t("explore.requestedShort")}
        </span>
      ) : (
        <Button
          size="sm"
          block
          icon={<UserPlus className="size-4" />}
          loading={request.isPending}
          onClick={() => request.mutate(user.id)}
        >
          {t("explore.add")}
        </Button>
      )}
    </li>
  );
}

/** The stamp store's most popular packs of the week. */
function PopularPacks() {
  const { t } = useTranslation("search");
  const packs = useQuery({
    queryKey: ["stamp", "store", "explore", "popular"],
    queryFn: () =>
      stampStoreClient().listStorePacks({
        section: stamp.StoreSection.POPULAR,
        page: { limit: 10 },
      }),
    staleTime: 5 * 60_000,
    retry: false,
  });
  const list = packs.data?.packs ?? [];
  if (list.length === 0) return null;
  return (
    <Block
      title={t("explore.packs")}
      icon={<Smile className="size-5 shrink-0 text-ink-soft" />}
      more={
        <Link
          to="/stamps"
          className="inline-flex shrink-0 items-center text-sm font-bold text-brand hover:underline"
        >
          {t("explore.more")}
          <ChevronRight className="size-4" />
        </Link>
      }
    >
      <ul className="scrollbar-none -mx-4 flex gap-3 overflow-x-auto px-4 pt-1 pb-2 md:mx-0 md:px-1 lg:grid lg:grid-cols-4 lg:overflow-visible lg:[&>li:nth-child(n+9)]:hidden">
        {list.map((pack) => {
          const cover =
            pack.cover?.variants
              .toSorted((a, b) => a.size - b.size)
              .find((variant) => variant.size >= 240)?.url ?? pack.cover?.url;
          return (
            <li key={pack.id} className="w-32 shrink-0 lg:w-auto">
              <Link
                to="/stamps/$packId"
                params={{ packId: pack.shortId || pack.id }}
                className="pressable sticker flex h-full flex-col gap-2 rounded-card bg-surface p-2.5"
              >
                <span className="grid aspect-square place-items-center overflow-hidden rounded-xl bg-surface-2">
                  {cover ? (
                    <img
                      src={cover}
                      alt=""
                      className="size-full object-contain p-1.5"
                      loading="lazy"
                      decoding="async"
                      draggable={false}
                    />
                  ) : (
                    <Smile className="size-8 text-ink-faint" />
                  )}
                </span>
                <span className="line-clamp-2 min-h-[2lh] text-sm leading-tight font-black break-words">
                  {pack.name}
                </span>
                {pack.price > 0 ? (
                  <Coins amount={pack.price} size="sm" />
                ) : (
                  <span className="w-fit rounded-full bg-lime px-2 py-0.5 text-xs font-black text-[#17122b]">
                    {t("hit.free")}
                  </span>
                )}
              </Link>
            </li>
          );
        })}
      </ul>
    </Block>
  );
}
