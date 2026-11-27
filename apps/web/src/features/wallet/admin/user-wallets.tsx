import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { admin, type common, wallet } from "@tokuly/api";
import { formatCoins, uniqueById, walletKeys } from "@tokuly/core/wallet";
import { ArrowLeft, Search, SlidersHorizontal } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Coins } from "#/components/ui/coins";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { api } from "#/lib/api";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { PAGE_SIZE, walletAdminClient } from "../api";
import { parseCoins } from "../format";
import { LedgerList } from "../ledger";
import { Fact, Stat } from "../parts";
import { AdjustmentRows } from "./adjustments";
import { useDebounced, useIsWide, useWalletStaffAccess } from "./hooks";
import { FilterChips, Pane, UserLine } from "./parts";

const userAdminClient = () => api().service(admin.UserAdminService);

/** Largest correction the API takes in one go, either way. */
const ADJUSTMENT_MAX = 100_000_000n;
const REASON_MAX = 500;

/**
 * Support view of anyone's wallet: find the account (by name, handle, email
 * or ID), then see its balances and ledger and, as an admin, correct a
 * balance. Search and wallet sit side by side on wide screens.
 */
export function UserWallets({
  userId,
  onSelect,
}: {
  userId?: string;
  onSelect(userId?: string): void;
}) {
  const { t } = useTranslation("wallet");
  const wide = useIsWide();
  const [text, setText] = useState("");
  const query = useDebounced(text.trim());
  const search = useQuery({
    queryKey: walletKeys.admin.userSearch(query),
    queryFn: () => userAdminClient().searchUsers({ query, page: { limit: query ? 25 : 10 } }),
  });
  const users = (search.data?.users ?? []).flatMap((found) =>
    found.user ? [{ ...found, user: found.user }] : [],
  );

  const finder = (
    <div className="min-w-0">
      <label className="flex h-12 items-center gap-2 rounded-control border-2 border-line bg-surface px-4 focus-within:border-brand">
        <Search className="size-5 shrink-0 text-ink-faint" />
        <input
          value={text}
          onChange={(event) => setText(event.target.value)}
          placeholder={t("staff.users.searchPlaceholder")}
          aria-label={t("staff.users.search")}
          maxLength={100}
          className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-ink-faint"
          autoCapitalize="none"
          spellCheck={false}
        />
      </label>
      <Pane className="mt-3" title={query ? t("staff.users.results") : t("staff.users.newest")}>
        {search.isPending ? (
          <div className="flex flex-col gap-2 p-3">
            {[0, 1, 2].map((row) => (
              <Skeleton key={row} className="h-12" />
            ))}
          </div>
        ) : search.isError ? (
          <EmptyState emoji="😵‍💫" title={errorMessage(search.error)} />
        ) : users.length > 0 ? (
          <ul className="divide-y-2 divide-line">
            {users.map((found) => (
              <li key={found.user.id}>
                <button
                  type="button"
                  onClick={() => onSelect(found.user.id)}
                  aria-current={found.user.id === userId ? "true" : undefined}
                  className={cn(
                    "flex w-full items-center gap-3 px-4 py-2.5 text-left transition-colors hover:bg-surface-2",
                    found.user.id === userId && "bg-brand-soft",
                  )}
                >
                  <Avatar user={found.user} size="sm" />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-black">
                      {found.user.deleted ? t("staff.deletedUser") : found.user.displayName}
                    </span>
                    <span className="block truncate text-xs text-ink-soft">
                      @{found.user.handle}
                      {found.email ? ` · ${found.email}` : ""}
                    </span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        ) : (
          <EmptyState
            emoji="🔍"
            title={t("staff.users.noResults")}
            body={t("staff.users.noResultsBody")}
          />
        )}
      </Pane>
    </div>
  );

  if (!wide) {
    // One thing at a time on a phone: the search, or the wallet that was picked.
    return userId ? (
      <div>
        <Button
          variant="ghost"
          size="sm"
          className="mb-3 -ml-2"
          icon={<ArrowLeft className="size-4" />}
          onClick={() => onSelect(undefined)}
        >
          {t("staff.users.backToSearch")}
        </Button>
        <UserWallet key={userId} userId={userId} />
      </div>
    ) : (
      finder
    );
  }

  return (
    <div className="grid items-start gap-4 lg:grid-cols-[minmax(0,4fr)_minmax(0,7fr)]">
      {finder}
      {userId ? (
        <UserWallet key={userId} userId={userId} />
      ) : (
        <Pane>
          <EmptyState emoji="👈" title={t("staff.users.pick")} body={t("staff.users.pickBody")} />
        </Pane>
      )}
    </div>
  );
}

type AccountFilter = "all" | "coins" | "earnings";

const ACCOUNTS: Record<AccountFilter, wallet.LedgerAccount> = {
  all: wallet.LedgerAccount.UNSPECIFIED,
  coins: wallet.LedgerAccount.COINS,
  earnings: wallet.LedgerAccount.EARNINGS,
};

/** One account's wallet: balances, lifetime figures, the ledger and (for admins) corrections. */
function UserWallet({ userId }: { userId: string }) {
  const { t } = useTranslation("wallet");
  const access = useWalletStaffAccess();
  const [filter, setFilter] = useState<AccountFilter>("all");
  const [adjusting, setAdjusting] = useState<{ attempt: number; open: boolean }>({
    attempt: 0,
    open: false,
  });

  const summary = useQuery({
    queryKey: walletKeys.admin.userWallet(userId),
    queryFn: () => walletAdminClient().getUserWallet({ userId }),
  });
  const account = ACCOUNTS[filter];
  const ledger = useInfiniteQuery({
    queryKey: walletKeys.admin.userLedger(userId, account),
    queryFn: ({ pageParam }) =>
      walletAdminClient().listUserLedgerEntries({
        userId,
        account,
        page: { limit: PAGE_SIZE, cursor: pageParam },
      }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const adjustments = useInfiniteQuery({
    queryKey: walletKeys.admin.adjustments(userId),
    queryFn: ({ pageParam }) =>
      walletAdminClient().listAdjustments({
        userId,
        page: { limit: PAGE_SIZE, cursor: pageParam },
      }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
    enabled: access.admin,
  });
  const entries = uniqueById(ledger.data?.pages.flatMap((page) => page.entries) ?? []);
  const corrections = uniqueById(adjustments.data?.pages.flatMap((page) => page.adjustments) ?? []);

  if (summary.isPending) return <Skeleton className="h-96 rounded-2xl" />;
  if (summary.isError) {
    return (
      <Pane>
        <EmptyState emoji="🔍" title={errorMessage(summary.error)} />
      </Pane>
    );
  }
  const data = summary.data;
  const balance = data.balance;

  return (
    <div className="flex min-w-0 flex-col gap-4">
      <Pane
        title={t("staff.users.walletTitle")}
        aside={
          access.admin && (
            <Button
              size="sm"
              variant="secondary"
              icon={<SlidersHorizontal className="size-4" />}
              onClick={() =>
                setAdjusting((previous) => ({ attempt: previous.attempt + 1, open: true }))
              }
            >
              {t("staff.adjust.open")}
            </Button>
          )
        }
      >
        <div className="p-4">
          <UserLine user={data.user} />
          <dl className="mt-4 grid grid-cols-2 gap-x-3 gap-y-4 sm:grid-cols-3">
            <Stat label={t("staff.users.coins")}>
              <Coins amount={balance?.coins ?? 0n} className="text-xl" />
            </Stat>
            <Stat label={t("staff.users.earnings")}>
              <Coins amount={balance?.earnings ?? 0n} className="text-xl" />
            </Stat>
            <Stat label={t("staff.users.pendingPayout")}>{formatCoins(data.pendingPayout)}</Stat>
            <Stat label={t("staff.users.lifetimePurchased")}>
              {formatCoins(data.lifetimePurchased)}
            </Stat>
            <Stat label={t("staff.users.lifetimeEarned")}>{formatCoins(data.lifetimeEarned)}</Stat>
            <Stat label={t("staff.users.lifetimePaidOut")}>
              {formatCoins(data.lifetimePaidOut)}
            </Stat>
          </dl>
        </div>
      </Pane>

      <Pane title={t("staff.users.ledger")}>
        <div className="border-b-2 border-line px-4 py-2.5">
          <FilterChips<AccountFilter>
            label={t("staff.users.ledgerFilter")}
            value={filter}
            onChange={setFilter}
            options={[
              { value: "all", label: t("history.filters.all") },
              { value: "coins", label: t("history.filters.coins") },
              { value: "earnings", label: t("history.filters.earnings") },
            ]}
          />
        </div>
        <div className="p-1.5">
          {ledger.isPending ? (
            <div className="flex flex-col gap-2 p-1.5">
              {[0, 1, 2].map((row) => (
                <Skeleton key={row} className="h-14" />
              ))}
            </div>
          ) : entries.length > 0 ? (
            <>
              <LedgerList entries={entries} grouped links={false} />
              <LoadMore query={ledger} />
            </>
          ) : (
            <EmptyState emoji="🪙" title={t("staff.users.ledgerEmpty")} />
          )}
        </div>
      </Pane>

      {access.admin && corrections.length > 0 && (
        <Pane title={t("staff.users.adjustments")}>
          <AdjustmentRows adjustments={corrections} showUser={false} />
          <LoadMore query={adjustments} />
        </Pane>
      )}

      {access.admin && adjusting.attempt > 0 && data.user && (
        <AdjustDialog
          key={adjusting.attempt}
          open={adjusting.open}
          onOpenChange={(open) => setAdjusting((previous) => ({ ...previous, open }))}
          user={data.user}
          coins={balance?.coins ?? 0n}
          earnings={balance?.earnings ?? 0n}
        />
      )}
    </div>
  );
}

/**
 * A manual correction of a balance: what, how much and why, then a second
 * look at exactly what will happen before it is carried out. Mounted fresh
 * per correction, which is also one idempotent attempt.
 */
function AdjustDialog({
  open,
  onOpenChange,
  user,
  coins,
  earnings,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  user: common.UserSummary;
  coins: bigint;
  earnings: bigint;
}) {
  const { t } = useTranslation(["wallet", "common"]);
  const queryClient = useQueryClient();
  const [idempotencyKey] = useState(() => crypto.randomUUID());
  const [account, setAccount] = useState<"coins" | "earnings">("coins");
  const [direction, setDirection] = useState<"add" | "remove">("add");
  const [amount, setAmount] = useState("");
  const [reason, setReason] = useState("");
  const [step, setStep] = useState<"form" | "review">("form");
  const [submitted, setSubmitted] = useState(false);

  const current = account === "coins" ? coins : earnings;
  const parsed = parseCoins(amount);
  const amountError =
    parsed === undefined || parsed <= 0n
      ? t("staff.adjust.amountInvalid")
      : parsed > ADJUSTMENT_MAX
        ? t("staff.adjust.amountTooLarge", { max: formatCoins(ADJUSTMENT_MAX) })
        : direction === "remove" && parsed > current
          ? t("staff.adjust.amountAboveBalance", { balance: formatCoins(current) })
          : undefined;
  const reasonError = reason.trim() ? undefined : t("staff.adjust.reasonRequired");
  const delta = (direction === "add" ? 1n : -1n) * (parsed ?? 0n);

  const adjust = useMutation({
    mutationFn: () =>
      walletAdminClient().adjustBalance({
        userId: user.id,
        account: account === "coins" ? wallet.LedgerAccount.COINS : wallet.LedgerAccount.EARNINGS,
        delta,
        reason: reason.trim(),
        idempotencyKey,
      }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: walletKeys.admin.userWallet(user.id) });
      void queryClient.invalidateQueries({ queryKey: ["wallet", "admin", "userLedger", user.id] });
      void queryClient.invalidateQueries({ queryKey: walletKeys.admin.allAdjustments });
      toast.success(t("staff.adjust.done"));
      onOpenChange(false);
    },
    onError: (error) => {
      toast.error(errorMessage(error));
      setStep("form");
      // The balance may have moved under us: show the current one.
      void queryClient.invalidateQueries({ queryKey: walletKeys.admin.userWallet(user.id) });
    },
  });

  const toReview = () => {
    setSubmitted(true);
    if (!amountError && !reasonError) setStep("review");
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={step === "form" ? t("staff.adjust.title") : t("staff.adjust.reviewTitle")}
      description={
        step === "form"
          ? t("staff.adjust.lead", { handle: user.handle })
          : t("staff.adjust.reviewLead")
      }
      footer={
        step === "form" ? (
          <>
            <Button variant="ghost" onClick={() => onOpenChange(false)}>
              {t("common:actions.cancel")}
            </Button>
            <Button onClick={toReview}>{t("staff.adjust.toReview")}</Button>
          </>
        ) : (
          <>
            <Button variant="ghost" onClick={() => setStep("form")} disabled={adjust.isPending}>
              {t("common:actions.back")}
            </Button>
            <Button
              variant={direction === "remove" ? "danger" : "primary"}
              loading={adjust.isPending}
              onClick={() => adjust.mutate()}
            >
              {t("staff.adjust.confirm")}
            </Button>
          </>
        )
      }
    >
      {step === "form" ? (
        <div className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <span className="px-1 text-sm font-bold text-ink-soft">
              {t("staff.adjust.account")}
            </span>
            <FilterChips<"coins" | "earnings">
              label={t("staff.adjust.account")}
              value={account}
              onChange={setAccount}
              options={[
                {
                  value: "coins",
                  label: t("staff.adjust.accountCoins", { balance: formatCoins(coins) }),
                },
                {
                  value: "earnings",
                  label: t("staff.adjust.accountEarnings", { balance: formatCoins(earnings) }),
                },
              ]}
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <span className="px-1 text-sm font-bold text-ink-soft">
              {t("staff.adjust.direction")}
            </span>
            <FilterChips<"add" | "remove">
              label={t("staff.adjust.direction")}
              value={direction}
              onChange={setDirection}
              options={[
                { value: "add", label: t("staff.adjust.add") },
                { value: "remove", label: t("staff.adjust.remove") },
              ]}
            />
          </div>
          <TextField
            label={t("staff.adjust.amount")}
            inputMode="numeric"
            autoComplete="off"
            value={amount}
            onChange={(event) => setAmount(event.target.value)}
            error={submitted ? amountError : undefined}
          />
          <TextArea
            label={t("staff.adjust.reason")}
            hint={t("staff.adjust.reasonHint")}
            value={reason}
            maxLength={REASON_MAX}
            onChange={(event) => setReason(event.target.value)}
            error={submitted ? reasonError : undefined}
          />
        </div>
      ) : (
        <dl className="divide-y-2 divide-line">
          <Fact label={t("staff.adjust.user")}>
            {user.displayName} <span className="text-ink-soft">@{user.handle}</span>
          </Fact>
          <Fact label={t("staff.adjust.account")}>
            {t(account === "coins" ? "history.filters.coins" : "history.filters.earnings")}
          </Fact>
          <Fact label={t("staff.adjust.change")}>
            <Coins amount={delta} signed className="text-xl" />
          </Fact>
          <Fact label={t("staff.adjust.result")}>
            {formatCoins(current)} →{" "}
            <span className="font-black">{formatCoins(current + delta)}</span>
          </Fact>
          <Fact label={t("staff.adjust.reason")}>
            <span className="whitespace-pre-wrap">{reason.trim()}</span>
          </Fact>
        </dl>
      )}
    </Dialog>
  );
}
