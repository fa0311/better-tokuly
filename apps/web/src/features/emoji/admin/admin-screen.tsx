import { useNavigate } from "@tanstack/react-router";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { EmptyState } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { useStaffAccess } from "#/features/admin";
import { AllEmoji } from "./all";
import { Categories } from "./categories";
import { Packs } from "./packs";
import { ReviewQueue, useReviewQueue } from "./queue";
import { TrustedSubmitters } from "./trusted";

const TABS = ["queue", "all", "categories", "trusted", "packs"] as const;
type Tab = (typeof TABS)[number];

/** What the staff page shows, as kept in the address: `/admin/emoji?tab=all&q=neko`. */
export interface AdminSearch {
  tab?: Tab;
  /** A name to look for on the "all" tab (the "manage" link on an emoji's page). */
  q?: string;
}

export function validateAdminSearch(search: Record<string, unknown>): AdminSearch {
  return {
    tab: TABS.find((tab) => tab === search.tab),
    q: typeof search.q === "string" ? search.q.slice(0, 64) : undefined,
  };
}

/**
 * The staff page for custom emoji, inside the admin console: the review
 * queue, every emoji with its edits, the categories, the trusted
 * submitters, and the import and export of whole sets.
 */
export function AdminScreen({ search }: { search: AdminSearch }) {
  const { t } = useTranslation("emoji");
  const navigate = useNavigate();
  const access = useStaffAccess();
  const tab = search.tab ?? "queue";

  return (
    <div className="h-full overflow-y-auto">
      <header className="glass sticky top-0 z-20 border-b-2 border-line">
        <div className="flex h-14 items-center gap-2 px-4">
          <h1 className="min-w-0 flex-1 truncate text-lg font-black">{t("admin.title")}</h1>
        </div>
      </header>
      <div className="px-4 pt-4 pb-32 md:pb-10">
        {access.moderator ? (
          <Moderation
            tab={tab}
            query={search.q ?? ""}
            onTab={(next) =>
              navigate({
                to: "/admin/emoji",
                search: { tab: next === "queue" ? undefined : next },
                replace: true,
              })
            }
          />
        ) : (
          <EmptyState emoji="🔐" title={t("admin.gateTitle")} body={t("admin.gateBody")} />
        )}
      </div>
    </div>
  );
}

function Moderation({ tab, query, onTab }: { tab: Tab; query: string; onTab(tab: Tab): void }) {
  const { t } = useTranslation("emoji");
  const queue = useReviewQueue();
  const waiting = queue.data?.pages[0]?.pendingCount;
  const strip = useRef<HTMLDivElement>(null);
  // On phones the tabs scroll sideways: keep the open one in view (also when arriving with `?tab=`).
  useEffect(() => {
    strip.current
      ?.querySelector('[role="tab"][aria-selected="true"]')
      ?.scrollIntoView({ block: "nearest", inline: "center" });
  }, [tab]);
  return (
    <>
      <div ref={strip}>
        <Tabs<Tab>
          value={tab}
          onChange={onTab}
          items={TABS.map((value) => ({
            value,
            label: t(`admin.tabs.${value}`),
            badge: value === "queue" ? waiting : undefined,
          }))}
        />
      </div>
      <div className="mt-4">
        {tab === "queue" && <ReviewQueue />}
        {tab === "all" && <AllEmoji initialQuery={query} />}
        {tab === "categories" && <Categories />}
        {tab === "trusted" && <TrustedSubmitters />}
        {tab === "packs" && <Packs />}
      </div>
    </>
  );
}
