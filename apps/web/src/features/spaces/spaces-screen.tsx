import type { InfiniteData, UseInfiniteQueryResult } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { spaces } from "@tokuly/api";
import type { SpacePage } from "@tokuly/core/spaces";
import { CalendarPlus, Mic, Plus } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { useBadge } from "#/stores/badges";
import { useLiveSpaces, useMySpaces, usePopularTags, useUpcomingSpaces } from "./api";
import { HangoutsTab } from "./hangout/hangouts-tab";
import { MySpaceActions } from "./my-space-actions";
import { FriendsInRoomsStrip } from "./presence";
import { ReminderButton } from "./reminder-button";
import { SpaceCard } from "./space-card";
import { SpaceForm } from "./space-form";
import type { SpacesTab } from "./spaces-search";

/** The way in to the audio rooms: what is on, what is coming, the person's hangouts and own rooms. */
export function SpacesScreen({ tab, tag }: { tab: SpacesTab; tag: string }) {
  const { t } = useTranslation("spaces");
  const navigate = useNavigate();
  const invitations = useBadge("spaces");
  const [creating, setCreating] = useState<"now" | "later" | null>(null);

  return (
    <Page
      title={t("title")}
      back="/me"
      wide
      actions={
        <Button
          size="sm"
          icon={<Mic className="size-4" />}
          onClick={() => setCreating("now")}
          data-testid="open-room"
        >
          {t("create")}
        </Button>
      }
    >
      <Tabs<SpacesTab>
        items={[
          { value: "discover", label: t("tabs.discover") },
          { value: "hangouts", label: t("tabs.hangouts"), badge: invitations },
          { value: "mine", label: t("tabs.mine") },
        ]}
        value={tab}
        onChange={(next) =>
          void navigate({
            to: "/spaces",
            search: next === "discover" ? {} : { tab: next },
            replace: true,
          })
        }
      />
      <div className="mt-5">
        {tab === "discover" && (
          <Discover
            tag={tag}
            onTag={(next) =>
              void navigate({ to: "/spaces", search: next ? { tag: next } : {}, replace: true })
            }
            onCreate={() => setCreating("now")}
          />
        )}
        {tab === "hangouts" && <HangoutsTab />}
        {tab === "mine" && <Mine onSchedule={() => setCreating("later")} />}
      </div>
      <SpaceForm
        open={creating !== null}
        onOpenChange={(open) => !open && setCreating(null)}
        schedule={creating === "later"}
      />
    </Page>
  );
}

type Language = "" | "ja" | "en";

function Discover({
  tag,
  onTag,
  onCreate,
}: {
  tag: string;
  onTag(tag: string): void;
  onCreate(): void;
}) {
  const { t } = useTranslation("spaces");
  const [language, setLanguage] = useState<Language>("");
  const [remindersOnly, setRemindersOnly] = useState(false);
  const friends = useLiveSpaces(spaces.LiveScope.FRIENDS);
  const trending = useLiveSpaces(spaces.LiveScope.PUBLIC, tag, language);
  const upcoming = useUpcomingSpaces(tag, language, remindersOnly);
  const tags = usePopularTags(language);
  const withFriends = friends.data?.pages.flatMap((page) => page.spaces) ?? [];
  // A tag that is being filtered by stays on offer, also when nothing is live under it.
  const offered = [
    ...(tag && !tags.data?.some((entry) => entry.tag === tag) ? [tag] : []),
    ...(tags.data?.map((entry) => entry.tag) ?? []),
  ];
  const chip = (active: boolean) =>
    cn(
      "inline-flex h-9 shrink-0 items-center rounded-full border-2 px-3.5 text-sm font-bold whitespace-nowrap transition-colors",
      active
        ? "border-ink bg-lime text-[#17122b]"
        : "border-line bg-surface text-ink-soft hover:text-ink",
    );

  return (
    <div className="flex flex-col gap-8">
      <FriendsInRoomsStrip />

      {withFriends.length > 0 && (
        <Section title={t("discover.friends")}>
          <Grid>
            {withFriends.map((space) => (
              <SpaceCard key={space.id} space={space} />
            ))}
          </Grid>
          <LoadMore query={friends} />
        </Section>
      )}

      <Section
        title={t("discover.trending")}
        filters={
          <fieldset className="scrollbar-none -mx-4 flex min-w-0 gap-2 overflow-x-auto px-4">
            <legend className="sr-only">{t("discover.filters")}</legend>
            {(["", "ja", "en"] as const).map((option) => (
              <button
                key={option || "all"}
                type="button"
                aria-pressed={language === option}
                onClick={() => setLanguage(option)}
                className={chip(language === option)}
              >
                {t(`language.${option || "all"}`)}
              </button>
            ))}
            {offered.length > 0 && (
              <span className="mx-1 w-0.5 shrink-0 self-stretch rounded-full bg-line" />
            )}
            {offered.map((entry) => (
              <button
                key={entry}
                type="button"
                aria-pressed={tag === entry}
                onClick={() => onTag(tag === entry ? "" : entry)}
                className={chip(tag === entry)}
              >
                #{entry}
              </button>
            ))}
          </fieldset>
        }
      >
        <SpaceList
          query={trending}
          empty={
            <EmptyState
              emoji="🎙️"
              title={tag ? t("discover.emptyTag", { tag }) : t("discover.empty")}
              body={t("discover.emptyBody")}
              action={
                <Button icon={<Mic className="size-4" />} onClick={onCreate}>
                  {t("create")}
                </Button>
              }
            />
          }
        />
      </Section>

      <Section
        title={t("discover.upcoming")}
        filters={
          <button
            type="button"
            aria-pressed={remindersOnly}
            onClick={() => setRemindersOnly(!remindersOnly)}
            className={cn(chip(remindersOnly), "self-start")}
          >
            {t("discover.remindersOnly")}
          </button>
        }
      >
        <SpaceList
          query={upcoming}
          action={(space) =>
            space.viewer && space.viewer.role !== spaces.SpaceRole.HOST ? (
              <ReminderButton space={space} />
            ) : null
          }
          empty={
            <p className="rounded-card border-2 border-dashed border-line px-4 py-8 text-center text-sm font-bold text-ink-faint">
              {t(remindersOnly ? "discover.noReminders" : "discover.noUpcoming")}
            </p>
          }
        />
      </Section>
    </div>
  );
}

function Mine({ onSchedule }: { onSchedule(): void }) {
  const { t } = useTranslation("spaces");
  const mine = useMySpaces();
  return (
    <div className="flex flex-col gap-4">
      <Button
        variant="secondary"
        className="self-start"
        icon={<CalendarPlus className="size-4" />}
        onClick={onSchedule}
      >
        {t("mine.schedule")}
      </Button>
      <SpaceList
        query={mine}
        action={(space) => <MySpaceActions space={space} />}
        empty={
          <EmptyState
            emoji="🪑"
            title={t("mine.empty")}
            body={t("mine.emptyBody")}
            action={
              <Button icon={<Plus className="size-4" />} onClick={onSchedule}>
                {t("mine.schedule")}
              </Button>
            }
          />
        }
      />
    </div>
  );
}

function Section({
  title,
  filters,
  children,
}: {
  title: string;
  filters?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-lg font-black">{title}</h2>
      {filters}
      {children}
    </section>
  );
}

function Grid({ children }: { children: ReactNode }) {
  return <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">{children}</div>;
}

function SpaceList({
  query,
  action,
  empty,
}: {
  query: UseInfiniteQueryResult<InfiniteData<SpacePage>>;
  action?(space: spaces.Space): ReactNode;
  empty: ReactNode;
}) {
  if (query.isPending)
    return (
      <Grid>
        <Skeleton className="h-40" />
        <Skeleton className="h-40" />
      </Grid>
    );
  const rooms = query.data?.pages.flatMap((page) => page.spaces) ?? [];
  if (rooms.length === 0) return empty;
  return (
    <>
      <Grid>
        {rooms.map((space) => (
          <SpaceCard key={space.id} space={space} action={action?.(space)} />
        ))}
      </Grid>
      <LoadMore query={query} />
    </>
  );
}
