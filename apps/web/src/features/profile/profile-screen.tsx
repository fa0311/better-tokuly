import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { common, identity, social, toDate } from "@tokuly/api";
import { Ban, LogOut, MessageCircle, Music, Settings, UserMinus, UserPlus } from "lucide-react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { ReportButton } from "#/features/engagement";
import { StoryRing } from "#/features/stories";
import { api, profileClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { formatDistance } from "#/lib/format";
import { forgetUser } from "#/seams/forget-user";
import { type ProfileSectionProps, ProfileSlotView, ProfileStat } from "#/seams/profile";
import { signOut as signOutOfApp } from "#/stores/session";

const friendClient = () => api().service(social.FriendService);

/** A person's profile: who they are, how you are connected, what you can do. */
export function ProfileScreen({ handle }: { handle: string }) {
  const { t, i18n } = useTranslation("profile");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const { data, isPending, isError } = useQuery({
    queryKey: ["identity", "profile", handle],
    queryFn: () => profileClient().getProfile({ by: { case: "handle", value: handle } }),
  });
  const profile = data?.profile;
  const user = profile?.user;
  const isSelf = profile?.relationship === identity.Relationship.SELF;
  const isFriend = profile?.relationship === identity.Relationship.FRIENDS;

  // Count the visit once per open profile (the server de-duplicates per day).
  const userId = user?.id;
  useEffect(() => {
    if (userId && !isSelf)
      void profileClient()
        .recordProfileView({ userId })
        .catch(() => {});
  }, [userId, isSelf]);

  const refresh = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: ["identity", "profile", handle] }),
      queryClient.invalidateQueries({ queryKey: ["social"] }),
    ]);
  const onError = (error: unknown) => toast.error(errorMessage(error));

  const addFriend = useMutation({
    mutationFn: () =>
      friendClient().sendFriendRequest({
        target: { case: "userId", value: user!.id },
        source: social.FriendSource.PROFILE,
      }),
    onSuccess: refresh,
    onError,
  });
  const removeFriend = useMutation({
    mutationFn: () => friendClient().removeFriend({ userId: user!.id }),
    onSuccess: () => {
      // What every feature loaded of theirs leaves the screen.
      forgetUser(queryClient, user!.id);
      return refresh();
    },
    onError,
  });
  const block = useMutation({
    mutationFn: () => friendClient().blockUser({ userId: user!.id }),
    onSuccess: () => {
      forgetUser(queryClient, user!.id);
      return navigate({ to: "/map" });
    },
    onError,
  });
  const signOut = async () => {
    await signOutOfApp();
    await navigate({ to: "/" });
  };

  if (isPending) {
    return (
      <div className="mx-auto max-w-2xl p-4">
        <Skeleton className="h-40" />
        <Skeleton className="mt-4 h-24" />
      </div>
    );
  }
  if (isError || !profile || !user) return <EmptyState emoji="🧭" title={t("notFound")} />;

  const stats = profile.stats;
  const joined = toDate(profile.joinedAt);
  const friendsSince = toDate(profile.friendsSince);
  const banner = profile.banner?.url;
  const tint = user.avatar?.dominantColor ?? "var(--brand)";
  // What every feature's profile section is handed (`src/features/<area>/profile.tsx`).
  const section: ProfileSectionProps = { profile, user, isSelf, isFriend };

  return (
    <div className="h-full overflow-y-auto pb-32 md:pb-10">
      {/* The banner falls back to the avatar's main colour, like the original app. */}
      <div
        className="h-40 w-full bg-cover bg-center md:h-52"
        style={banner ? { backgroundImage: `url(${banner})` } : { background: tint }}
      />
      <div className="mx-auto -mt-14 max-w-2xl px-4">
        {/* While the person has stories to show, their face is ringed and plays them. */}
        <StoryRing user={user}>
          <Avatar
            user={user}
            size="xl"
            outlined
            className="block [&>span]:size-28 [&>span]:rounded-[2rem]"
          />
        </StoryRing>
        <h1 className="mt-3 truncate text-2xl font-black">
          <EmojiText text={user.displayName} /> {user.verified && <span title="verified">✅</span>}
        </h1>
        <p className="truncate font-bold text-ink-soft">
          @{user.handle}
          {profile.extras?.pronouns && (
            <span className="ml-2 text-sm font-bold text-ink-faint">{profile.extras.pronouns}</span>
          )}
        </p>
        {profile.bot && (
          <p className="mt-2 inline-flex flex-wrap items-center gap-2 text-sm font-bold">
            <span className="rounded-full border-2 border-ink bg-sky px-2.5 py-0.5 text-xs font-black text-[#17122b]">
              🤖 {t("bot")}
            </span>
            {profile.botOwner && (
              <Link
                to="/u/$handle"
                params={{ handle: profile.botOwner.handle }}
                className="text-ink-soft underline-offset-4 hover:underline"
              >
                {t("botOwner", { handle: profile.botOwner.handle })}
              </Link>
            )}
          </p>
        )}

        {(profile.status?.emoji || profile.status?.text) && (
          <p className="sticker-sm mt-4 inline-flex max-w-full items-center gap-2 rounded-full bg-surface px-4 py-1.5 text-sm font-bold">
            <span>{profile.status.emoji}</span>
            <span className="truncate">
              <EmojiText text={profile.status.text} />
            </span>
          </p>
        )}
        {profile.bio && (
          <p className="mt-4 leading-relaxed whitespace-pre-wrap">
            <EmojiText text={profile.bio} />
          </p>
        )}

        {/* What features say about the person right now (勉強中, the class that is on). */}
        <ProfileSlotView slot="status" {...section} />

        <div className="mt-5 flex flex-wrap gap-2">
          {isSelf ? (
            <>
              <Link
                to="/settings"
                className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand"
              >
                <Settings className="size-4" />
                {t("actions.settings")}
              </Link>
              <ProfileSlotView slot="action" {...section} />
              <Button variant="secondary" icon={<LogOut className="size-4" />} onClick={signOut}>
                {t("actions.signOut")}
              </Button>
            </>
          ) : (
            <>
              {isFriend ? (
                <Link
                  to="/chat"
                  search={{ user: user.id }}
                  className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand"
                >
                  <MessageCircle className="size-4" />
                  {t("actions.message")}
                </Link>
              ) : profile.relationship === identity.Relationship.REQUEST_SENT ? (
                <Button variant="secondary" disabled>
                  {t("actions.requested")}
                </Button>
              ) : (
                <Button
                  icon={<UserPlus className="size-4" />}
                  loading={addFriend.isPending}
                  onClick={() => addFriend.mutate()}
                >
                  {profile.relationship === identity.Relationship.REQUEST_RECEIVED
                    ? t("actions.accept")
                    : t("actions.add")}
                </Button>
              )}
              <ProfileSlotView slot="action" {...section} />
              {isFriend && (
                <Button
                  variant="ghost"
                  icon={<UserMinus className="size-4" />}
                  onClick={() =>
                    window.confirm(t("confirmRemove", { name: user.displayName })) &&
                    removeFriend.mutate()
                  }
                >
                  {t("actions.remove")}
                </Button>
              )}
              <Button
                variant="ghost"
                icon={<Ban className="size-4" />}
                onClick={() =>
                  window.confirm(t("confirmBlock", { name: user.displayName })) && block.mutate()
                }
              >
                {t("actions.block")}
              </Button>
              <ReportButton
                subject={{ type: common.SubjectType.USER, id: user.id }}
                label={`@${user.handle}`}
              />
            </>
          )}
        </div>

        {/* Cards of features: ひま right now, countdowns, highlights. */}
        <ProfileSlotView slot="card" {...section} />

        {profile.distanceMeters !== undefined && (
          <p className="mt-5 rounded-2xl bg-lime px-4 py-3 font-black text-[#17122b] sticker-sm">
            {t("distance", { distance: formatDistance(profile.distanceMeters) })}
          </p>
        )}

        {stats && (
          <Card className="mt-5 grid grid-cols-3 gap-y-4 text-center sm:grid-cols-6">
            <ProfileStat value={stats.friendCount} label={t("stats.friends")} />
            <ProfileStat value={stats.introCount} label={t("stats.intros")} />
            <ProfileStat value={Number(stats.profileViewCount)} label={t("stats.views")} />
            <ProfileStat value={stats.bumpCount} label={t("stats.bumps")} />
            <ProfileSlotView slot="stat" {...section} />
          </Card>
        )}

        {profile.nowPlaying?.title && (
          <a
            href={profile.nowPlaying.trackUrl || undefined}
            target="_blank"
            rel="noreferrer"
            className="sticker mt-5 flex items-center gap-3 rounded-card bg-surface p-3"
          >
            {profile.nowPlaying.artworkUrl ? (
              <img
                src={profile.nowPlaying.artworkUrl}
                alt=""
                className="size-14 rounded-xl object-cover"
              />
            ) : (
              <div className="grid size-14 place-items-center rounded-xl bg-mint text-[#17122b]">
                <Music className="size-6" />
              </div>
            )}
            <div className="min-w-0 flex-1">
              <p className="text-xs font-black text-success">{t("nowPlaying")}</p>
              <p className="truncate font-black">{profile.nowPlaying.title}</p>
              <p className="truncate text-sm text-ink-soft">{profile.nowPlaying.artist}</p>
            </div>
          </a>
        )}

        <div className="mt-5 flex flex-col gap-1 text-sm font-bold text-ink-soft">
          {joined && (
            <p>🎂 {t("joined", { name: user.displayName, count: stats?.daysSinceJoined ?? 0 })}</p>
          )}
          {isFriend && friendsSince && (
            <p>🤝 {t("friendsSince", { date: friendsSince.toLocaleDateString(i18n.language) })}</p>
          )}
          {profile.bestFriend && <p>💜 {t("bestFriend")}</p>}
          {!isSelf && profile.mutualFriendCount > 0 && (
            <p className="flex items-center gap-2">
              <span className="flex -space-x-2">
                {profile.mutualFriends.slice(0, 5).map((friend) => (
                  <Avatar key={friend.id} user={friend} size="xs" />
                ))}
              </span>
              {t("mutual", { count: profile.mutualFriendCount })}
            </p>
          )}
        </div>

        {/* Nothing of a blocked person is shown, and the server would not hand it out. */}
        {profile.relationship !== identity.Relationship.BLOCKED && (
          <ProfileSlotView slot="content" {...section} />
        )}
      </div>
    </div>
  );
}
