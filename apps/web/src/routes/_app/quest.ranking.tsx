import { createFileRoute } from "@tanstack/react-router";
import { RankingScreen, type RankingSearch } from "#/features/quest/ranking-screen";

const PERIODS = ["week", "season", "allTime"] as const;

export const Route = createFileRoute("/_app/quest/ranking")({
  // The open board and period live in the address: `/quest/ranking?board=streak`.
  validateSearch: (search: Record<string, unknown>): RankingSearch => ({
    board: typeof search.board === "string" && search.board ? search.board : undefined,
    period: PERIODS.find((period) => period === search.period),
  }),
  component: RankingPage,
});

function RankingPage() {
  const search = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <RankingScreen
      search={search}
      onSearch={(next) => void navigate({ search: next, replace: true })}
    />
  );
}
