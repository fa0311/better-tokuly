import { isMember } from "@tokuly/core/community";
import { useTranslation } from "react-i18next";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { EmojiText } from "#/features/emoji";
import { Facts, Rules } from "./about";
import { NS, useAnnouncements } from "./api";
import { Boards } from "./boards";
import { useCurrentCommunity } from "./context";
import { EventsTab } from "./events";
import { CommunityHeader } from "./header";
import { HOME_TABS, type HomeSearch } from "./home-search";
import { MembersTab } from "./members";
import { ThreadCard } from "./thread-card";

/** A community's front page: the header, then its boards, events, announcements, members and rules. */
export function HomeScreen({
  search,
  onSearchChange,
}: {
  search: HomeSearch;
  onSearchChange(next: HomeSearch): void;
}) {
  const { t } = useTranslation(NS);
  const c = useCurrentCommunity();
  const tab = search.tab ?? "boards";
  // The directory is for members; everything else follows the community.
  const tabs = HOME_TABS.filter((each) => each !== "members" || isMember(c));
  return (
    <Page title={<EmojiText text={c.name} />} back="/communities" wide>
      <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_20rem]">
        <div className="min-w-0">
          <CommunityHeader community={c} />
          <Tabs
            className="mt-4"
            items={tabs.map((value) => ({ value, label: t(`tabs.${value}`) }))}
            value={tab}
            onChange={(next) => onSearchChange(next === "boards" ? {} : { tab: next })}
          />
          <div className="mt-3">
            {tab === "boards" && (
              <Boards
                community={c}
                search={search}
                onSearchChange={(next) => onSearchChange({ ...next, tab: undefined })}
              />
            )}
            {tab === "events" && <EventsTab community={c} />}
            {tab === "news" && <AnnouncementsTab />}
            {tab === "members" && isMember(c) && <MembersTab community={c} />}
            {tab === "about" && (
              <div className="flex flex-col gap-4">
                <Rules community={c} />
                <Facts community={c} />
              </div>
            )}
          </div>
        </div>
        {tab !== "about" && (
          <aside className="hidden flex-col gap-4 lg:flex">
            <Rules community={c} />
            <Facts community={c} />
          </aside>
        )}
      </div>
    </Page>
  );
}

function AnnouncementsTab() {
  const { t } = useTranslation(NS);
  const c = useCurrentCommunity();
  const announcements = useAnnouncements(c.id);
  if (announcements.isPending) return <Skeleton className="h-28" />;
  if (announcements.items?.length === 0)
    return <EmptyState emoji="📣" title={t("news.empty")} body={t("news.emptyBody")} />;
  return (
    <div className="flex flex-col gap-2">
      {announcements.items?.map((thread) => (
        <ThreadCard key={thread.id} thread={thread} handle={c.handle} />
      ))}
      <LoadMore query={announcements} />
    </div>
  );
}
