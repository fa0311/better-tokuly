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

/**
 * The shorts of a person on their profile. A channel does not say whose it
 * is, so this exists where the API tells: on the viewer's own profile (their
 * default channel) and on a friend's (the friends feed names the friend).
 */
function ProfileShorts({ user, isSelf, isFriend }: ProfileSectionProps) {
  const { t } = useTranslation("shorts");
  const mine = useMyChannels();
  const myHandle = isSelf ? (mine.data?.channels[0]?.summary?.handle ?? "") : "";
  const query = useQuery({
    queryKey: ["shorts", "profile", user.id, myHandle],
    queryFn: async (): Promise<shorts.Short[]> => {
      if (isSelf) {
        const response = await discoveryClient().listChannelShorts({
          channelId: myHandle,
          page: { limit: SHOWN, cursor: "" },
        });
        return response.shorts;
      }
      const response = await feedClient().getFriendsFeed({ page: { limit: 30, cursor: "" } });
      return response.items.flatMap((item) =>
        item.short && item.friend?.id === user.id ? [item.short] : [],
      );
    },
    enabled: isSelf ? Boolean(myHandle) : isFriend,
    staleTime: 60_000,
  });
  const list = query.data ?? [];
  const handle = list[0]?.channel?.handle;
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
