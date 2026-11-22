import { Link } from "@tanstack/react-router";
import { Bookmark, CalendarHeart, History, Sparkles } from "lucide-react";
import { type ReactNode, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { EmptyState } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { openCompose } from "./compose-store";
import { useHomeFeed } from "./feed/api";
import { HomeFeed, hasMoreThanNotes } from "./feed/home-feed";
import { ComposeButton, NotesPage, useScroller } from "./shell";
import { Timeline, type TimelineHandle, TimelineSkeleton } from "./timeline";
import { TrendingHashtags } from "./trending";

export const NOTES_TABS = ["feed", "home", "global", "mentions"] as const;
export type NotesTab = (typeof NOTES_TABS)[number];

/** Reads `/notes?tab=…`; anything else means "the default tab". */
export function parseNotesSearch(search: Record<string, unknown>): { tab?: NotesTab } {
  const tab = NOTES_TABS.find((value) => value === search.tab);
  return tab ? { tab } : {};
}

/**
 * The notes home: the mixed feed of what friends posted and did (when there
 * is more to it than notes), the friends timeline, everyone's public notes
 * and the notes that mention the viewer, with trends beside them.
 */
export function NotesScreen({
  tab: chosen,
  onTabChange,
  top,
}: {
  tab?: NotesTab;
  onTabChange(
    tab: NotesTab,
  ): void /** Shown above the list on the feed and friends tabs: the story tray. */;
  top?: ReactNode;
}) {
  const { t } = useTranslation("notes");
  const timeline = useRef<TimelineHandle>(null);
  const feed = useHomeFeed();
  const mixed = hasMoreThanNotes(feed.data?.pages[0]?.cards ?? []);
  // The feed tab exists only when the feed has more than notes; until that is known, nothing is picked for the viewer.
  const tab: NotesTab | undefined =
    chosen === "feed" && !mixed && !feed.isPending
      ? "home"
      : (chosen ?? (feed.isPending ? undefined : mixed ? "feed" : "home"));

  const items = [
    ...(mixed || chosen === "feed" ? [{ value: "feed" as const, label: t("tabs.feed") }] : []),
    { value: "home" as const, label: t("tabs.home") },
    { value: "global" as const, label: t("tabs.global") },
    { value: "mentions" as const, label: t("tabs.mentions") },
  ];
  const writeFirst = (
    <Button variant="lime" onClick={() => openCompose()}>
      {t("empty.write")}
    </Button>
  );

  return (
    <NotesPage
      title={t("title")}
      scrollId="notes-home"
      onRefresh={() => timeline.current?.refresh() ?? Promise.resolve()}
      actions={
        <Link
          to="/bookmarks"
          aria-label={t("shell.bookmarks")}
          className="grid size-9 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2 hover:text-ink lg:hidden"
        >
          <Bookmark className="size-5" />
        </Link>
      }
      toolbar={<TabsRow items={items} value={tab ?? "home"} onChange={onTabChange} />}
      aside={
        <div className="sticky top-32 flex flex-col gap-4">
          <TrendingHashtags layout="panel" />
          <Shortcuts />
        </div>
      }
    >
      {(tab === "feed" || tab === "home") && top}
      {tab === "global" && (
        <div className="lg:hidden">
          <TrendingHashtags layout="strip" />
        </div>
      )}
      {tab === undefined && <TimelineSkeleton />}
      {tab === "feed" && (
        <HomeFeed
          ref={timeline}
          empty={
            <EmptyState
              emoji="🫧"
              title={t("empty.feed")}
              body={t("empty.feedBody")}
              action={writeFirst}
            />
          }
        />
      )}
      {tab === "home" && (
        <Timeline
          key="home"
          ref={timeline}
          source={{ kind: "home" }}
          live
          empty={
            <EmptyState
              emoji="📝"
              title={t("empty.home")}
              body={t("empty.homeBody")}
              action={writeFirst}
            />
          }
        />
      )}
      {tab === "global" && (
        <Timeline
          key="global"
          ref={timeline}
          source={{ kind: "global" }}
          empty={
            <EmptyState
              emoji="🌏"
              title={t("empty.global")}
              body={t("empty.globalBody")}
              action={writeFirst}
            />
          }
        />
      )}
      {tab === "mentions" && (
        <Timeline
          key="mentions"
          ref={timeline}
          source={{ kind: "mentions" }}
          live
          empty={
            <EmptyState emoji="💬" title={t("empty.mentions")} body={t("empty.mentionsBody")} />
          }
        />
      )}
      <ComposeButton />
    </NotesPage>
  );
}

/** The tab strip. Switching tabs starts the new list from its top. */
function TabsRow({
  items,
  value,
  onChange,
}: {
  items: { value: NotesTab; label: string }[];
  value: NotesTab;
  onChange(tab: NotesTab): void;
}) {
  const { element } = useScroller();
  return (
    <Tabs<NotesTab>
      items={items}
      value={value}
      onChange={(next) => {
        element?.scrollTo({ top: 0 });
        onChange(next);
      }}
    />
  );
}

/** Ways into the viewer's own corners, for wide screens (phones find them on the "me" page). */
function Shortcuts() {
  const { t } = useTranslation("notes");
  const links = [
    { to: "/bookmarks", label: t("shortcuts.bookmarks"), icon: Bookmark, tint: "bg-sky" },
    { to: "/activity", label: t("shortcuts.activity"), icon: History, tint: "bg-mint" },
    { to: "/stories/archive", label: t("shortcuts.archive"), icon: CalendarHeart, tint: "bg-pink" },
    { to: "/recap", label: t("shortcuts.recap"), icon: Sparkles, tint: "bg-sun" },
  ] as const;
  return (
    <nav aria-label={t("shortcuts.title")} className="rounded-card bg-surface p-2">
      {links.map((link) => (
        <Link
          key={link.to}
          to={link.to}
          className="flex items-center gap-3 rounded-2xl px-2 py-2 font-bold hover:bg-surface-2"
        >
          <span
            className={`grid size-8 place-items-center rounded-xl border-2 border-ink text-[#17122b] ${link.tint}`}
          >
            <link.icon className="size-4" />
          </span>
          {link.label}
        </Link>
      ))}
    </nav>
  );
}
