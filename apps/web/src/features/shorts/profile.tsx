import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { FeedContext } from "@tokuly/core/shorts";
import { ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { ProfileSection, ProfileSectionProps } from "#/seams/profile";
import { listQuery } from "./lists";
import { ShortTile } from "./short-tile";

/** How many shorts a profile shows before "see all". */
const SHOWN = 6;

/**
 * The shorts of a person on their profile, across their channels, as the
 * viewer may see them (`ListUserShorts`: friends-only ones for friends,
 * nothing for someone in a block relation). A tile opens the swipe feed
 * over the same list. "See all" leads to the channel when they all come
 * from one.
 */
function ProfileShorts({ user }: ProfileSectionProps) {
  const { t } = useTranslation("shorts");
  const context: FeedContext = { kind: "user", key: user.id };
  const query = useInfiniteQuery({ ...listQuery(context), staleTime: 60_000 });
  const list = (query.data?.pages ?? []).flatMap((page) => page.entries).slice(0, SHOWN);
  if (list.length === 0) return null;
  const handles = new Set(list.map(({ short }) => short.channel?.handle ?? ""));
  const handle = handles.size === 1 ? ([...handles][0] ?? "") : "";

  return (
    <section className="mt-5" aria-labelledby="profile-shorts" data-testid="profile-shorts">
      <div className="mb-2 flex items-center justify-between gap-2 px-1">
        <h2 id="profile-shorts" className="font-black">
          {t("profile.title")}
        </h2>
        {handle && (
          <Link
            to="/c/$handle/$tab"
            params={{ handle, tab: "shorts" }}
            className="flex items-center text-sm font-bold text-ink-soft hover:text-ink"
          >
            {t("profile.all")}
            <ChevronRight className="size-4" />
          </Link>
        )}
      </div>
      <div className="scrollbar-none -mx-4 flex gap-2 overflow-x-auto px-4">
        {list.map(({ short }) => (
          <ShortTile key={short.id} short={short} context={context} className="w-28 shrink-0" />
        ))}
      </div>
    </section>
  );
}

export default [
  { id: "shorts", slot: "content", order: 30, Component: ProfileShorts },
] satisfies ProfileSection[];
