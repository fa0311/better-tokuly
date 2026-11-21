import { Link } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { Coins } from "#/components/ui/coins";
import { Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { formatCoins } from "./format";
import { useBalance } from "./use-balance";

/**
 * The signed-in user's coins as a small pill that opens the wallet. Meant
 * for headers of screens where coins are spent (stamp store, live gifts);
 * it stays current on its own.
 */
export function BalancePill({ className }: { className?: string }) {
  const { t } = useTranslation("wallet");
  const { coins } = useBalance();
  return (
    <Link
      to="/wallet"
      aria-label={coins === undefined ? t("pill.open") : t("pill.label", { coins: formatCoins(coins) })}
      className={cn("pressable sticker-sm inline-flex h-9 shrink-0 items-center rounded-full bg-surface pr-3 pl-2", className)}
    >
      {coins === undefined ? <Skeleton className="h-4 w-12 rounded-full" /> : <Coins amount={coins} size="sm" />}
    </Link>
  );
}
