import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { errorReason, type stamp } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Coins } from "#/components/ui/coins";
import { Dialog } from "#/components/ui/dialog";
import { Skeleton } from "#/components/ui/page";
import { useBalance } from "#/features/wallet";
import { errorMessage } from "#/lib/errors";
import { libraryClient, patchPack, refreshHoldings, stampKeys } from "./api";
import { PackCover, PackMaker } from "./parts";

/**
 * Asks before spending coins on a paid pack: shows the price next to the
 * balance and what will be left, and sends people who are short to the
 * wallet instead.
 */
export function PurchaseDialog({ pack, open, onOpenChange }: { pack: stamp.PackSummary; open: boolean; onOpenChange(open: boolean): void }) {
  const { t } = useTranslation("stamps");
  const queryClient = useQueryClient();
  // Kept current by the wallet: coins bought in another tab show up here on their own.
  const { coins, refresh: refreshBalance } = useBalance();
  const price = BigInt(pack.price);
  const short = coins !== undefined && coins < price ? price - coins : 0n;

  const purchase = useMutation({
    mutationFn: () => libraryClient().purchasePack({ packId: pack.id, expectedPrice: pack.price }),
    onSuccess: (response) => {
      const bought = response.pack;
      if (bought) patchPack(queryClient, bought.id, () => bought);
      toast.success(t("toast.purchased", { name: pack.name }));
      onOpenChange(false);
      return Promise.all([refreshHoldings(queryClient), refreshBalance()]);
    },
    onError: (error) => {
      toast.error(errorMessage(error));
      const reason = errorReason(error);
      // The price or the balance on screen is out of date: show the real ones.
      if (reason === "PRICE_CHANGED") void queryClient.invalidateQueries({ queryKey: stampKeys.all });
      if (reason === "INSUFFICIENT_COINS") void refreshBalance();
    },
  });

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => !purchase.isPending && onOpenChange(next)}
      title={t("purchase.title")}
      description={t("purchase.lead")}
      footer={
        <>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={purchase.isPending}>
            {t("purchase.cancel")}
          </Button>
          {short > 0n ? (
            <Link to="/wallet/buy" className="pressable sticker inline-flex h-11 items-center justify-center rounded-full bg-lime px-5 text-[15px] font-bold text-[#17122b]">
              {t("purchase.topUp")}
            </Link>
          ) : (
            <Button onClick={() => purchase.mutate()} loading={purchase.isPending} disabled={coins === undefined}>
              {t("purchase.confirm")}
            </Button>
          )}
        </>
      }
    >
      <div className="flex items-center gap-3">
        <PackCover pack={pack} className="size-16 shrink-0" />
        <div className="min-w-0">
          <p className="truncate text-lg font-black">{pack.name}</p>
          <PackMaker pack={pack} className="text-sm" />
        </div>
      </div>

      <dl className="mt-5 flex flex-col gap-2.5 rounded-2xl bg-surface-2 p-4 font-bold">
        <div className="flex items-center justify-between">
          <dt className="text-ink-soft">{t("purchase.price")}</dt>
          <dd data-testid="purchase-price">
            <Coins amount={price} />
          </dd>
        </div>
        <div className="flex items-center justify-between">
          <dt className="text-ink-soft">{t("purchase.balance")}</dt>
          <dd data-testid="purchase-balance">{coins === undefined ? <Skeleton className="h-6 w-16" /> : <Coins amount={coins} />}</dd>
        </div>
        {coins !== undefined && short === 0n && (
          <div className="flex items-center justify-between border-t-2 border-line pt-2.5">
            <dt className="text-ink-soft">{t("purchase.after")}</dt>
            <dd data-testid="purchase-after">
              <Coins amount={coins - price} />
            </dd>
          </div>
        )}
      </dl>

      {short > 0n && (
        <p role="alert" className="mt-4 rounded-2xl border-2 border-danger px-4 py-3 text-sm font-bold text-danger">
          {t("purchase.short", { count: Number(short) })}
        </p>
      )}
    </Dialog>
  );
}
