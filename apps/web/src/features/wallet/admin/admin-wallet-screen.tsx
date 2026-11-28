import { useQuery } from "@tanstack/react-query";
import { wallet } from "@tokuly/api";
import { walletKeys } from "@tokuly/core/wallet";
import { EmptyState } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { useTranslation } from "#/lib/use-translation";
import { walletAdminClient } from "../api";
import { AdjustmentsLog } from "./adjustments";
import { STAFF_TABS, type StaffTab, type StaffWalletSearch } from "./admin-wallet-search";
import { useWalletStaffAccess } from "./hooks";
import { PackageManager } from "./packages";
import { PayoutQueue } from "./payout-queue";
import { UserWallets } from "./user-wallets";

/**
 * The staff side of the wallet, inside the admin console: the payout review
 * queue and wallet lookups for moderators; balance adjustments, their log and
 * the coin shop for admins.
 */
export function AdminWalletScreen({
  search,
  onSearch,
}: {
  search: StaffWalletSearch;
  onSearch(next: StaffWalletSearch): void;
}) {
  const { t } = useTranslation("wallet");
  const access = useWalletStaffAccess();
  // The number waiting for review rides on the tab, whichever tab is open.
  const pending = useQuery({
    queryKey: walletKeys.admin.pendingCount,
    queryFn: async () =>
      (
        await walletAdminClient().listPayoutQueue({
          status: wallet.PayoutStatus.PENDING,
          page: { limit: 1 },
        })
      ).pendingCount,
    enabled: access.moderator,
  });

  const tabs: readonly StaffTab[] = access.admin ? STAFF_TABS : ["payouts", "users"];
  const tab = search.tab && tabs.includes(search.tab) ? search.tab : "payouts";

  return (
    <div className="h-full overflow-y-auto">
      <header className="glass sticky top-0 z-20 border-b-2 border-line">
        <div className="flex h-14 items-center gap-2 px-4">
          <h1 className="min-w-0 flex-1 truncate text-lg font-black">{t("staff.title")}</h1>
        </div>
      </header>
      <div className="px-4 pt-4 pb-32 md:pb-10">
        {!access.moderator ? (
          <EmptyState emoji="🔐" title={t("staff.gate.title")} body={t("staff.gate.body")} />
        ) : (
          <>
            <Tabs<StaffTab>
              value={tab}
              onChange={(next) => onSearch({ tab: next })}
              items={tabs.map((value) => ({
                value,
                label: t(`staff.tabs.${value}`),
                badge: value === "payouts" ? pending.data : undefined,
              }))}
            />
            <div className="mt-4">
              {tab === "payouts" && (
                <PayoutQueue
                  selectedId={search.payout}
                  onSelect={(payout) => onSearch({ tab: "payouts", payout })}
                  onOpenUser={(user) => onSearch({ tab: "users", user })}
                />
              )}
              {tab === "users" && (
                <UserWallets
                  userId={search.user}
                  onSelect={(user) => onSearch({ tab: "users", user })}
                />
              )}
              {tab === "adjustments" && (
                <AdjustmentsLog onOpenUser={(user) => onSearch({ tab: "users", user })} />
              )}
              {tab === "packages" && <PackageManager />}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
