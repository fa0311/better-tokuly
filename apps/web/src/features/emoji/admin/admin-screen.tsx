import { useNavigate } from "@tanstack/react-router";
import { useEffect, useRef } from "react";
import { EmptyState } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { useStaffAccess } from "#/features/admin";
import { useTranslation } from "#/lib/use-translation";
import { ADMIN_TABS, type AdminSearch, type AdminTab } from "./admin-search";
import { AllEmoji } from "./all";
import { Categories } from "./categories";
import { Packs } from "./packs";
import { ReviewQueue, useReviewQueue } from "./queue";
import { TrustedSubmitters } from "./trusted";

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

function Moderation({
  tab,
  query,
  onTab,
}: {
  tab: AdminTab;
  query: string;
  onTab(tab: AdminTab): void;
}) {
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
        <Tabs<AdminTab>
          value={tab}
          onChange={onTab}
          items={ADMIN_TABS.map((value) => ({
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
