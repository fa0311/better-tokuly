import { createFileRoute } from "@tanstack/react-router";
import {
  AdminWalletScreen,
  STAFF_TABS,
  type StaffTab,
  type StaffWalletSearch,
} from "#/features/wallet/admin/admin-wallet-screen";

export const Route = createFileRoute("/_app/admin/wallet")({
  // The open tab, account and payout request live in the address:
  // `/admin/wallet?tab=users&user=<id>` opens that account's wallet.
  validateSearch: (search: Record<string, unknown>): StaffWalletSearch => ({
    tab: STAFF_TABS.includes(search.tab as StaffTab) ? (search.tab as StaffTab) : undefined,
    user: typeof search.user === "string" && search.user ? search.user : undefined,
    payout: typeof search.payout === "string" && search.payout ? search.payout : undefined,
  }),
  component: AdminWalletPage,
});

function AdminWalletPage() {
  const search = Route.useSearch();
  const navigate = Route.useNavigate();
  return <AdminWalletScreen search={search} onSearch={(next) => void navigate({ search: next })} />;
}
