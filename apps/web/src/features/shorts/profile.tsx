import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { shorts } from "@tokuly/api";
import { ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useMyChannels } from "#/features/video";
import type { ProfileSection, ProfileSectionProps } from "#/seams/profile";
import { discoveryClient, feedClient } from "./api";
import { ShortTile } from "./short-tile";

/** How many shorts a profile shows before "see all". */
const SHOWN = 6;
/** How far into the viewer's friends feed the friend's channel is looked for. */
const SEARCH_PAGES = 3;

/**
 * The handle of a friend's channel. Channels do not say whose they are, so
 * it is read off the friends feed, which names the friend on each item:
 * page by page until one of theirs turns up (or a few pages without one).
 */
async function friendChannelHandle(userId: string): Promise<string> {
  let cursor = "";
  for (let page = 0; page < SEARCH_PAGES; page++) {
    const response = await feedClient().getFriendsFeed({ page: { limit: 30, cursor } });
    const item = response.items.find((entry) => entry.friend?.id === userId && entry.short);
    if (item?.short?.channel?.handle) return item.short.channel.handle;
    cursor = response.page?.nextCursor ?? "";
    if (!cursor) break;
  }
  return "";
}

/**
 * The shorts of a person on their profile. On the viewer's own profile they
 * come from their default channel; on a friend's, from the friend's channel
 * once the friends feed has given it away (there is no "shorts by user"
 * yet), so a friend whose shorts are not in the viewer's recent friends feed
 * shows none here.
 */
function ProfileShorts({ user, isSelf, isFriend }: ProfileSectionProps) {
  const { t } = useTranslation("shorts");
  const mine = useMyChannels();
  const handleQuery = useQuery({
    queryKey: ["shorts", "profile", "channel", user.id],
    queryFn: () => friendChannelHandle(user.id),
    enabled: !isSelf && isFriend,
    staleTime: 10 * 60_000,
  });
  const handle = isSelf
    ? (mine.data?.channels[0]?.summary?.handle ?? "")
    : (handleQuery.data ?? "");
  const query = useQuery({
    queryKey: ["shorts", "profile", "shorts", handle],
    queryFn: async (): Promise<shorts.Short[]> =>
      (
        await discoveryClient().listChannelShorts({
          channelId: handle,
          page: { limit: SHOWN, cursor: "" },
        })
      ).shorts,
    enabled: Boolean(handle),
    staleTime: 60_000,
  });
  const list = query.data ?? [];
  if (list.length === 0 || !handle) return null;

  return (
    <section className="mt-5" aria-labelledby="profile-shorts" data-testid="profile-shorts">
      <div className="mb-2 flex items-center justify-between gap-2 px-1">
        <h2 id="profile-shorts" className="font-black">
          {t("profile.title")}
        </h2>
        <Link
          to="/c/$handle/$tab"
          params={{ handle, tab: "shorts" }}
          className="flex items-center text-sm font-bold text-ink-soft hover:text-ink"
        >
          {t("profile.all")}
          <ChevronRight className="size-4" />
        </Link>
      </div>
      <div className="scrollbar-none -mx-4 flex gap-2 overflow-x-auto px-4">
        {list.slice(0, SHOWN).map((short) => (
          <ShortTile
            key={short.id}
            short={short}
            context={{ kind: "channel", key: handle }}
            className="w-28 shrink-0"
          />
        ))}
      </div>
    </section>
  );
}

export default [
  { id: "shorts", slot: "content", order: 30, Component: ProfileShorts },
] satisfies ProfileSection[];
