import { Link } from "@tanstack/react-router";
import { Lock } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import type { ProfileSection, ProfileSectionProps } from "#/seams/profile";
import { ProfileStat } from "#/seams/profile";
import { FollowButton } from "./follow-button";
import { summaryFromProfile, useFollowSummary } from "./follow-data";

/** The profile's follow numbers and relation: the card's values first, then the live summary. */
function useProfileFollows({ profile, user }: ProfileSectionProps) {
  return useFollowSummary(user.id, summaryFromProfile(profile)).data;
}

/** フォローする / フォロー中 / リクエスト済み / フォローバック. */
function FollowAction(props: ProfileSectionProps) {
  const summary = useProfileFollows(props);
  if (props.isSelf || !summary) return null;
  return (
    <FollowButton user={props.user} relation={summary.relation} locked={summary.locked} size="md" />
  );
}

/** Two cells of the numbers card: followers and following, each opening its list. */
function FollowStats(props: ProfileSectionProps) {
  const { t } = useTranslation("follows");
  const summary = useProfileFollows(props);
  if (!summary) return null;
  const handle = props.user.handle;
  return (
    <>
      <Link
        to="/u/$handle/followers"
        params={{ handle }}
        className="rounded-2xl hover:bg-surface-2"
        data-testid="profile-followers"
      >
        <ProfileStat value={summary.followerCount} label={t("stats.followers")} />
      </Link>
      <Link
        to="/u/$handle/following"
        params={{ handle }}
        className="rounded-2xl hover:bg-surface-2"
        data-testid="profile-following"
      >
        <ProfileStat value={summary.followingCount} label={t("stats.following")} />
      </Link>
    </>
  );
}

/** "フォローされています", the lock, and "followed by A, B and N others". */
function FollowContext(props: ProfileSectionProps) {
  const { t } = useTranslation("follows");
  const summary = useProfileFollows(props);
  if (!summary || props.isSelf) return null;
  const known = summary.knownFollowers;
  const others = Math.max(0, summary.knownFollowerCount - known.length);
  const followsYou = summary.relation?.followsYou;
  if (!followsYou && !summary.locked && known.length === 0) return null;
  return (
    <div className="mt-4 flex flex-col gap-2">
      {(followsYou || summary.locked) && (
        <div className="flex flex-wrap gap-2">
          {followsYou && (
            <span className="sticker-sm inline-flex h-7 items-center rounded-full bg-sky px-3 text-xs font-black text-[#17122b]">
              {t("tag.followsYou")}
            </span>
          )}
          {summary.locked && (
            <span className="sticker-sm inline-flex h-7 items-center gap-1 rounded-full bg-surface px-3 text-xs font-black">
              <Lock className="size-3.5" aria-hidden />
              {t("tag.locked")}
            </span>
          )}
        </div>
      )}
      {known.length > 0 && (
        <Link
          to="/u/$handle/known-followers"
          params={{ handle: props.user.handle }}
          className="flex items-center gap-2 text-sm font-bold text-ink-soft hover:text-ink"
          data-testid="profile-known-followers"
        >
          <span className="flex -space-x-2">
            {known.slice(0, 3).map((follower) => (
              <Avatar key={follower.id} user={follower} size="xs" outlined />
            ))}
          </span>
          <span className="min-w-0">
            {others > 0
              ? t("known.withOthers", {
                  names: known.map((follower) => follower.displayName).join(t("known.separator")),
                  count: others,
                })
              : t("known.only", {
                  names: known.map((follower) => follower.displayName).join(t("known.separator")),
                })}
          </span>
        </Link>
      )}
    </div>
  );
}

export default [
  { id: "friends-follow", slot: "action", order: 5, Component: FollowAction },
  { id: "friends-follow-context", slot: "status", order: 5, Component: FollowContext },
  { id: "friends-follow-stats", slot: "stat", order: 5, Component: FollowStats },
] satisfies ProfileSection[];
