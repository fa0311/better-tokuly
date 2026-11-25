import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { errorReason, toMillis } from "@tokuly/api";
import { profileKeys } from "@tokuly/core/profile";
import { Send } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { ShareToChat } from "#/features/chat";
import { EmojiText } from "#/features/emoji";
import { useAccountTimeZone, zonedDate } from "#/lib/account-time";
import { errorMessage } from "#/lib/errors";
import { badgeClient, questKeys, useLocalized, useUserBadges } from "./api";
import { formatDay, formatShare, fraction, isHiddenSecret, tierKey } from "./model";
import { BadgeMedal, ProgressBar, RarityChip, TitlePill, useBadgeName } from "./parts";

/** How many badges a showcase holds. */
export const SHOWCASE_SIZE = 5;

/**
 * `/quest/badges/<key>`: one badge — how it is earned, how rare it is, how
 * far the viewer is, which friends have it — and, once earned, putting it on
 * the profile and wearing its title.
 */
export function BadgeScreen({ badgeKey }: { badgeKey: string }) {
  const { t, i18n } = useTranslation("quest");
  const text = useLocalized();
  const badgeName = useBadgeName();
  const queryClient = useQueryClient();
  const zone = useAccountTimeZone();
  const { data, isPending, error } = useQuery({
    queryKey: questKeys.badge(badgeKey),
    queryFn: () => badgeClient().getBadge({ key: badgeKey }),
    retry: (count, failure) => errorReason(failure) !== "BADGE_NOT_FOUND" && count < 2,
  });
  const mine = useUserBadges("");
  const [sharing, setSharing] = useState(false);
  const badge = data?.badge;

  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: questKeys.badges });
    void queryClient.invalidateQueries({ queryKey: questKeys.progress });
    void queryClient.invalidateQueries({ queryKey: questKeys.profiles });
    void queryClient.invalidateQueries({ queryKey: profileKeys.profiles });
  };
  const onError = (failure: unknown) => toast.error(errorMessage(failure, { namespace: "quest" }));

  const showcaseKeys = mine.data?.showcase.map((item) => item.key) ?? [];
  const shown = showcaseKeys.includes(badgeKey);
  const setShowcase = useMutation({
    mutationFn: (keys: string[]) => badgeClient().setShowcase({ badgeKeys: keys }),
    onSuccess: (_, keys) => {
      refresh();
      toast.success(t(keys.includes(badgeKey) ? "badge.shown" : "badge.hidden"));
    },
    onError,
  });
  const wearing = mine.data?.title?.badgeKey === badgeKey;
  const setTitle = useMutation({
    mutationFn: (key: string) => badgeClient().setTitle({ badgeKey: key }),
    onSuccess: (_, key) => {
      refresh();
      toast.success(t(key ? "badge.titleOn" : "badge.titleOff"));
    },
    onError,
  });

  if (isPending)
    return (
      <Page title={t("badge.title")} back="/quest/badges">
        <Skeleton className="h-72" />
      </Page>
    );
  if (!badge)
    return (
      <Page title={t("badge.title")} back="/quest/badges">
        <EmptyState
          emoji="🫥"
          title={
            errorReason(error) === "BADGE_NOT_FOUND" || !error
              ? t("badge.notFound")
              : errorMessage(error, { namespace: "quest" })
          }
          body={t("badge.notFoundBody")}
          action={
            <Link to="/quest/badges" className="font-black text-brand">
              {t("badge.toCollection")}
            </Link>
          }
        />
      </Page>
    );

  const secret = isHiddenSecret(badge);
  const name = badgeName(badge);
  const title = text(badge.title);
  const tier = tierKey(badge.tier);
  // The day it was earned, on the account's calendar.
  const earned = badge.earnedAt ? zonedDate(toMillis(badge.earnedAt), zone) : undefined;
  const counted = badge.target > 0n;
  const full = !shown && showcaseKeys.length >= SHOWCASE_SIZE;

  return (
    <Page
      title={name}
      back="/quest/badges"
      actions={
        !secret && (
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t("badge.share")}
            onClick={() => setSharing(true)}
          >
            <Send className="size-5" />
          </Button>
        )
      }
    >
      <section className="sticker flex flex-col items-center rounded-card bg-surface px-5 pt-8 pb-6 text-center">
        <BadgeMedal badge={badge} size="xl" className="animate-pop-in" />
        <h2 className="mt-6 text-2xl font-black" data-testid="quest-badge-name">
          {name}
        </h2>
        <div className="mt-2 flex flex-wrap items-center justify-center gap-2">
          {tier && (
            <span className="rounded-full bg-surface-2 px-2.5 py-0.5 text-xs font-black text-ink-soft">
              {t("badge.tier", { tier: t(`tiers.${tier}`) })}
            </span>
          )}
          <RarityChip rarity={badge.rarity} />
          {badge.ownerShare > 0 && (
            <span className="text-xs font-bold text-ink-soft">
              {t("badge.ownerShare", { share: formatShare(badge.ownerShare) })}
            </span>
          )}
        </div>
        <p className="mt-4 max-w-sm leading-relaxed text-ink-soft">
          {secret ? t("badges.secretHint") : text(badge.description)}
        </p>

        {title && (
          <p className="mt-4 flex flex-wrap items-center justify-center gap-2 text-sm font-bold">
            {t("badge.unlocksTitle")}
            <TitlePill title={title} className="h-6 px-2 text-xs" />
          </p>
        )}

        {badge.owned ? (
          <p className="mt-4 rounded-full bg-lime px-4 py-1.5 text-sm font-black text-[#17122b]">
            {earned
              ? t("badge.earnedOn", { date: formatDay(earned, i18n.language, true) })
              : t("badge.earned")}
            {badge.times > 1 && ` ${t("badge.times", { count: badge.times })}`}
          </p>
        ) : (
          <p className="mt-4 rounded-full bg-surface-2 px-4 py-1.5 text-sm font-black text-ink-soft">
            {t("badge.locked")}
          </p>
        )}

        {counted && !badge.owned && (
          <div className="mt-5 w-full max-w-sm">
            <ProgressBar
              value={fraction(badge.progress, badge.target)}
              label={t("badges.progressLabel", { name })}
              valueText={`${badge.progress} / ${badge.target}`}
            />
            <p className="mt-1 text-sm font-black tabular-nums">
              {Number(badge.progress).toLocaleString()} / {Number(badge.target).toLocaleString()}
            </p>
          </div>
        )}

        {badge.owned && (
          <div className="mt-6 flex flex-wrap justify-center gap-2">
            <Button
              variant={shown ? "secondary" : "lime"}
              loading={setShowcase.isPending}
              disabled={mine.isPending || full}
              onClick={() =>
                setShowcase.mutate(
                  shown
                    ? showcaseKeys.filter((key) => key !== badgeKey)
                    : [...showcaseKeys, badgeKey],
                )
              }
            >
              {shown ? t("badge.hide") : t("badge.show")}
            </Button>
            {title && (
              <Button
                variant={wearing ? "secondary" : "primary"}
                loading={setTitle.isPending}
                disabled={mine.isPending}
                onClick={() => setTitle.mutate(wearing ? "" : badgeKey)}
              >
                {wearing ? t("badge.removeTitle") : t("badge.wearTitle", { title })}
              </Button>
            )}
          </div>
        )}
        {badge.owned && full && (
          <p className="mt-2 text-sm font-bold text-ink-soft">
            {t("badge.showcaseFull", { count: SHOWCASE_SIZE })}{" "}
            <Link to="/quest/showcase" className="text-brand underline">
              {t("badges.editShowcase")}
            </Link>
          </p>
        )}
      </section>

      <section aria-label={t("badge.friends")} className="mt-6">
        <h2 className="text-lg font-black">{t("badge.friends")}</h2>
        {data.friendOwnerCount === 0 ? (
          <p className="mt-2 text-sm font-bold text-ink-soft">
            {badge.owned ? t("badge.friendsNoneOwned") : t("badge.friendsNone")}
          </p>
        ) : (
          <ul className="sticker mt-3 rounded-card bg-surface p-2">
            {data.friendOwners.map((friend) => (
              <li key={friend.id}>
                <Link
                  to="/quest/u/$handle"
                  params={{ handle: friend.handle }}
                  className="flex items-center gap-3 rounded-2xl px-3 py-2 transition-colors hover:bg-surface-2"
                >
                  <Avatar user={friend} size="sm" />
                  <span className="min-w-0 flex-1 truncate font-bold">
                    <EmojiText text={friend.displayName} />
                  </span>
                </Link>
              </li>
            ))}
            {data.friendOwnerCount > data.friendOwners.length && (
              <li className="px-3 py-2 text-sm font-bold text-ink-soft">
                {t("badge.friendsMore", {
                  count: data.friendOwnerCount - data.friendOwners.length,
                })}
              </li>
            )}
          </ul>
        )}
      </section>

      {/* Sent as a link: the friend sees the badge with how far they are themselves. */}
      <ShareToChat
        open={sharing}
        onOpenChange={setSharing}
        share={{ url: `${window.location.origin}/quest/badges/${badge.key}` }}
        title={name}
      />
    </Page>
  );
}
