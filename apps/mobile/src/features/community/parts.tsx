import { type common, community, toMillis } from "@tokuly/api";
import { categoryOf, imageUrl, pseudonymName, ROLE_KEYS, safeColor } from "@tokuly/core/community";
import { compactAge } from "@tokuly/core/format";
import { Image } from "expo-image";
import { router } from "expo-router";
import { Lock, Megaphone, MessageCircle, Pin } from "lucide-react-native";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { mediaUrl } from "#/lib/config";
import { useTheme } from "#/lib/theme";
import { NS } from "./data";

const ICON_PIXELS = { sm: 40, md: 52, lg: 76 } as const;

/** A community's picture, or the emoji of its category on a tile. */
export function CommunityIcon({
  community: c,
  size = "md",
}: {
  community: Pick<community.Community, "icon" | "category">;
  size?: keyof typeof ICON_PIXELS;
}) {
  const pixels = ICON_PIXELS[size];
  const url = mediaUrl(imageUrl(c.icon, pixels));
  return (
    <View
      className="items-center justify-center overflow-hidden bg-surface-3"
      style={{ width: pixels, height: pixels, borderRadius: Math.round(pixels * 0.3) }}
    >
      {url ? (
        <Image source={url} style={{ width: "100%", height: "100%" }} contentFit="cover" />
      ) : (
        <Text style={{ fontSize: Math.round(pixels * 0.5) }}>{categoryOf(c.category).emoji}</Text>
      )}
    </View>
  );
}

/** "123人" in the app's language. */
export function useMemberCount(): (count: number) => string {
  const { t, i18n } = useTranslation(NS);
  return (count) =>
    t("memberCount", { formatted: new Intl.NumberFormat(i18n.language).format(count) });
}

/** A community in a list: its icon, name, category and members; a dot when something is unread. */
export function CommunityRow({
  community: c,
  testID,
}: {
  community: community.Community;
  testID?: string;
}) {
  const { t } = useTranslation(NS);
  const members = useMemberCount();
  const category = categoryOf(c.category);
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      onPress={() => router.push(`/g/${c.handle}`)}
      className="flex-row items-center gap-3 border-b border-line bg-surface px-4 py-3 active:bg-surface-2"
    >
      <CommunityIcon community={c} />
      <View className="flex-1 gap-0.5">
        <Text numberOfLines={1} className="text-base font-black text-ink">
          {c.name}
        </Text>
        <Text numberOfLines={1} className="text-xs text-ink-soft">
          {`${category.emoji} ${t(`category.${category.key}`)} · ${members(c.memberCount)}`}
        </Text>
      </View>
      {c.viewer?.hasUnread && (
        <View accessibilityLabel={t("unread")} className="size-2.5 rounded-full bg-pink" />
      )}
    </Pressable>
  );
}

/** "オーナー" / "管理者" / "モデレーター"; nothing for a plain member. */
export function RoleChip({ role }: { role: community.MemberRole }) {
  const { t } = useTranslation(NS);
  const key = ROLE_KEYS[role];
  if (key === "member") return null;
  const tone = key === "owner" ? "bg-sun" : key === "admin" ? "bg-sky" : "bg-mint";
  return (
    <View className={`rounded-full px-2 py-0.5 ${tone}`}>
      <Text className="text-[11px] font-black text-[#17122b]">{t(`role.${key}`)}</Text>
    </View>
  );
}

/** A small label next to a thread's title (pinned, announcement, locked, waiting for review). */
function Flag({ icon, label }: { icon?: ReactNode; label: string }) {
  return (
    <View className="flex-row items-center gap-1 rounded-full bg-surface-2 px-2 py-0.5">
      {icon}
      <Text className="text-[11px] font-black text-ink-soft">{label}</Text>
    </View>
  );
}

/**
 * Who wrote a post: the person (avatar, name), or in an anonymous channel
 * the thread's pseudonym on its colour. Nothing else names the author of an
 * anonymous post; the author alone sees "あなた".
 */
export function Byline({
  author,
  pseudonym,
  mine,
  at,
  extra,
}: {
  author?: common.UserSummary;
  pseudonym?: community.Pseudonym;
  mine?: boolean;
  at?: community.Thread["createdAt"];
  extra?: ReactNode;
}) {
  const { t, i18n } = useTranslation(NS);
  const name = pseudonym
    ? pseudonymName(pseudonym, i18n.language)
    : author?.deleted
      ? t("post.deletedUser")
      : author?.displayName || author?.handle || "";
  return (
    <View className="flex-row items-center gap-2">
      {pseudonym ? (
        <View
          className="size-8 items-center justify-center rounded-lg bg-surface-3"
          style={
            safeColor(pseudonym.color) ? { backgroundColor: safeColor(pseudonym.color) } : null
          }
        >
          <Text className="text-base">{pseudonym.emoji || "🫥"}</Text>
        </View>
      ) : (
        <Pressable
          accessibilityRole="button"
          disabled={!author?.handle || author.deleted}
          onPress={() => author && router.push(`/u/${author.handle}`)}
        >
          <Avatar user={author} size="sm" />
        </Pressable>
      )}
      <View className="flex-1 flex-row flex-wrap items-center gap-x-1.5">
        <Text numberOfLines={1} className="shrink text-sm font-black text-ink">
          {name}
        </Text>
        {mine && <Text className="text-xs font-bold text-brand">{t("post.mine")}</Text>}
        {extra}
        {at && <Text className="text-xs text-ink-faint">{compactAge(i18n, toMillis(at))}</Text>}
      </View>
    </View>
  );
}

/** One thread on a board: its flags, title, a first look at the text and its counts. */
export function ThreadCard({ thread, handle }: { thread: community.Thread; handle: string }) {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const unread = thread.viewer?.unreadReplyCount ?? 0;
  return (
    <Pressable
      testID={`thread-${thread.shortId}`}
      accessibilityRole="button"
      onPress={() => router.push(`/g/${handle}/t/${thread.shortId}`)}
      className="mx-4 mb-3 gap-2 rounded-card bg-surface p-4 active:bg-surface-2"
    >
      {(thread.pinned ||
        thread.announcement ||
        thread.locked ||
        thread.state === community.PostState.HIDDEN) && (
        <View className="flex-row flex-wrap gap-1.5">
          {thread.pinned && (
            <Flag icon={<Pin size={11} color={colors["ink-soft"]} />} label={t("thread.pinned")} />
          )}
          {thread.announcement && (
            <Flag
              icon={<Megaphone size={11} color={colors["ink-soft"]} />}
              label={t("thread.announcement")}
            />
          )}
          {thread.locked && (
            <Flag icon={<Lock size={11} color={colors["ink-soft"]} />} label={t("thread.locked")} />
          )}
          {thread.state === community.PostState.HIDDEN && <Flag label={t("thread.hidden")} />}
        </View>
      )}
      <Byline
        author={thread.author}
        pseudonym={thread.pseudonym}
        mine={thread.viewer?.mine}
        at={thread.lastActivityAt}
      />
      <Text numberOfLines={2} className="text-base font-black text-ink">
        {thread.title}
      </Text>
      {thread.body !== "" && (
        <Text numberOfLines={2} className="text-sm leading-5 text-ink-soft">
          {thread.body}
        </Text>
      )}
      <View className="flex-row items-center gap-3">
        <View className="flex-row items-center gap-1">
          <MessageCircle size={14} color={colors["ink-soft"]} />
          <Text className="text-xs font-bold text-ink-soft">
            {t("thread.replies", { count: thread.replyCount })}
          </Text>
        </View>
        {unread > 0 && (
          <Text className="text-xs font-black text-pink">
            {t("thread.unreadReplies", { count: unread })}
          </Text>
        )}
        {thread.poll && (
          <Text className="text-xs font-bold text-ink-soft">📊 {t("poll.title")}</Text>
        )}
      </View>
    </Pressable>
  );
}
