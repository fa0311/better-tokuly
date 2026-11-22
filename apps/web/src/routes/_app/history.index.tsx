import { createFileRoute } from "@tanstack/react-router";
import { HistoryScreen, type HistoryTab } from "#/features/history/history-screen";

const TABS: readonly string[] = ["visits", "nights", "stats"];

// `/history?tab=visits|nights|stats`: the calendar, every visit, the nights, the numbers.
export const Route = createFileRoute("/_app/history/")({
  validateSearch: (search: Record<string, unknown>): { tab?: Exclude<HistoryTab, "calendar"> } =>
    typeof search.tab === "string" && TABS.includes(search.tab) ? { tab: search.tab as Exclude<HistoryTab, "calendar"> } : {},
  component: HistoryPage,
});

function HistoryPage() {
  const { tab } = Route.useSearch();
  return <HistoryScreen tab={tab ?? "calendar"} />;
}
