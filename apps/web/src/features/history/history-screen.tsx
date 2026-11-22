import { useNavigate } from "@tanstack/react-router";
import { EllipsisVertical, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Menu } from "#/components/ui/menu";
import { Page } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { HistoryCalendar } from "./calendar";
import { DeleteHistoryDialog } from "./delete-history";
import { NightsList } from "./nights-list";
import { StatsPanel } from "./stats-panel";
import { VisitsList } from "./visits-list";

export type HistoryTab = "calendar" | "visits" | "nights" | "stats";

/** `/history`: the diary nobody had to write — by day, by visit, by night, in numbers. */
export function HistoryScreen({ tab }: { tab: HistoryTab }) {
  const { t } = useTranslation("history");
  const navigate = useNavigate();
  const [deleting, setDeleting] = useState(false);

  return (
    <Page
      title={t("title")}
      back="/me"
      wide
      actions={
        <Menu
          trigger={
            <Button variant="ghost" size="icon-sm" aria-label={t("more")}>
              <EllipsisVertical className="size-5" />
            </Button>
          }
          actions={[{ label: t("delete.action"), icon: <Trash2 className="size-4" />, danger: true, onSelect: () => setDeleting(true) }]}
        />
      }
    >
      <Tabs
        items={[
          { value: "calendar", label: t("tabs.calendar") },
          { value: "visits", label: t("tabs.visits") },
          { value: "nights", label: t("tabs.nights") },
          { value: "stats", label: t("tabs.stats") },
        ]}
        value={tab}
        onChange={(next) => void navigate({ to: "/history", search: next === "calendar" ? {} : { tab: next }, replace: true })}
      />
      <div className="mt-4">
        {tab === "calendar" && <HistoryCalendar />}
        {tab === "visits" && <VisitsList />}
        {tab === "nights" && <NightsList />}
        {tab === "stats" && <StatsPanel />}
      </div>
      <DeleteHistoryDialog open={deleting} onOpenChange={setDeleting} />
    </Page>
  );
}
