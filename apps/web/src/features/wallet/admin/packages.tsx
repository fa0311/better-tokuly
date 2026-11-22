import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { fromDate, type Timestamp, toDate, toMillis, wallet } from "@tokuly/api";
import { Pencil, Plus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { SwitchRow } from "#/components/ui/switch";
import { errorMessage } from "#/lib/errors";
import { walletAdminClient, walletKeys } from "../api";
import { formatCoins, formatMoney, parseCoins } from "../format";
import { StatusChip, type Tone } from "../parts";
import { Cell, ColumnHeads, Pane, RECORD_ROW } from "./parts";

const BADGE_KEYS = {
  none: wallet.PackageBadge.UNSPECIFIED,
  popular: wallet.PackageBadge.POPULAR,
  bestValue: wallet.PackageBadge.BEST_VALUE,
  limited: wallet.PackageBadge.LIMITED,
} as const;

type BadgeKey = keyof typeof BADGE_KEYS;

function badgeKey(badge: wallet.PackageBadge): BadgeKey {
  return (Object.keys(BADGE_KEYS) as BadgeKey[]).find((key) => BADGE_KEYS[key] === badge) ?? "none";
}

/** Whether buyers can see a package right now, and if not, why. */
function saleState(pkg: wallet.CoinPackage, now: number): { key: string; tone: Tone } {
  if (!pkg.active) return { key: "retired", tone: "neutral" };
  const from = toMillis(pkg.availableFrom);
  const until = toMillis(pkg.availableUntil);
  if (from && now < from) return { key: "scheduled", tone: "info" };
  if (until && now >= until) return { key: "ended", tone: "neutral" };
  return { key: "onSale", tone: "good" };
}

const COLUMNS =
  "minmax(10rem,1.6fr) 4.5rem 4.5rem 5.5rem 4.5rem 3.5rem minmax(0,1fr) 5.5rem 10.5rem";

/** The coin shop's shelf as staff sees it: every package, on sale or not, with create / edit / retire (admins only). */
export function PackageManager() {
  const { t, i18n } = useTranslation("wallet");
  const queryClient = useQueryClient();
  const [editing, setEditing] = useState<{
    pkg: wallet.CoinPackage | null;
    attempt: number;
    open: boolean;
  }>({ pkg: null, attempt: 0, open: false });
  // The package being retired or put back. It stays set while the confirmation closes, so the dialog keeps its words.
  const [toggling, setToggling] = useState<{ pkg: wallet.CoinPackage; open: boolean } | null>(null);
  const { data, isPending, isError, error } = useQuery({
    queryKey: walletKeys.admin.packages,
    queryFn: () => walletAdminClient().listAllCoinPackages({}),
  });

  const save = useMutation({
    mutationFn: (pkg: wallet.CoinPackage) => walletAdminClient().saveCoinPackage({ package: pkg }),
    onSuccess: (_response, pkg) => {
      void queryClient.invalidateQueries({ queryKey: walletKeys.admin.packages });
      void queryClient.invalidateQueries({ queryKey: walletKeys.packages });
      toast.success(
        t(pkg.active ? "staff.packages.toast.restored" : "staff.packages.toast.retired"),
      );
      setToggling((current) => current && { ...current, open: false });
    },
    onError: (failure) => toast.error(errorMessage(failure)),
  });

  const open = (pkg: wallet.CoinPackage | null) =>
    setEditing((previous) => ({ pkg, attempt: previous.attempt + 1, open: true }));
  const now = Date.now();
  const date = (timestamp: Timestamp | undefined) =>
    toDate(timestamp)?.toLocaleString(i18n.language, { dateStyle: "short", timeStyle: "short" });
  const heads = ["sku", "coins", "bonus", "price", "badge", "order", "window", "state"].map((key) =>
    t(`staff.packages.columns.${key}`),
  );

  return (
    <Pane
      title={t("staff.packages.title")}
      aside={
        <Button size="sm" icon={<Plus className="size-4" />} onClick={() => open(null)}>
          {t("staff.packages.create")}
        </Button>
      }
    >
      {isPending ? (
        <div className="flex flex-col gap-2 p-3">
          {[0, 1, 2].map((row) => (
            <Skeleton key={row} className="h-12" />
          ))}
        </div>
      ) : isError ? (
        <EmptyState emoji="😵‍💫" title={errorMessage(error)} />
      ) : data.packages.length > 0 ? (
        <div>
          <ColumnHeads columns={COLUMNS} heads={[...heads, ""]} />
          <ul className="divide-y-2 divide-line">
            {data.packages.map((pkg) => {
              const state = saleState(pkg, now);
              const window =
                pkg.availableFrom || pkg.availableUntil
                  ? `${date(pkg.availableFrom) ?? "…"} 〜 ${date(pkg.availableUntil) ?? "…"}`
                  : t("staff.packages.always");
              return (
                <li key={pkg.id} className={RECORD_ROW} style={{ gridTemplateColumns: COLUMNS }}>
                  <Cell full label={heads[0] ?? ""}>
                    <span className="font-mono font-bold break-all">{pkg.sku}</span>
                  </Cell>
                  <Cell label={heads[1] ?? ""}>{formatCoins(pkg.coins)}</Cell>
                  <Cell label={heads[2] ?? ""}>
                    {pkg.bonusCoins > 0n ? `+${formatCoins(pkg.bonusCoins)}` : "—"}
                  </Cell>
                  <Cell label={heads[3] ?? ""}>
                    {formatMoney(pkg.priceMinor, pkg.currency, i18n.language)}
                  </Cell>
                  <Cell label={heads[4] ?? ""}>
                    {pkg.badge === wallet.PackageBadge.UNSPECIFIED
                      ? "—"
                      : t(`shop.badge.${badgeKey(pkg.badge)}`)}
                  </Cell>
                  <Cell label={heads[5] ?? ""}>{pkg.sortOrder}</Cell>
                  <Cell label={heads[6] ?? ""}>{window}</Cell>
                  <Cell label={heads[7] ?? ""}>
                    <StatusChip tone={state.tone}>
                      {t(`staff.packages.state.${state.key}`)}
                    </StatusChip>
                  </Cell>
                  <div className="mt-1 flex w-full gap-2 lg:mt-0 lg:w-auto lg:justify-end">
                    <Button
                      size="sm"
                      variant="secondary"
                      icon={<Pencil className="size-3.5" />}
                      aria-label={t("staff.packages.editLabel", { sku: pkg.sku })}
                      onClick={() => open(pkg)}
                    >
                      {t("staff.packages.edit")}
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      aria-label={t(
                        pkg.active ? "staff.packages.retireLabel" : "staff.packages.restoreLabel",
                        { sku: pkg.sku },
                      )}
                      onClick={() => setToggling({ pkg, open: true })}
                    >
                      {t(pkg.active ? "staff.packages.retire" : "staff.packages.restore")}
                    </Button>
                  </div>
                </li>
              );
            })}
          </ul>
        </div>
      ) : (
        <EmptyState
          emoji="📦"
          title={t("staff.packages.empty")}
          body={t("staff.packages.emptyBody")}
        />
      )}

      {editing.attempt > 0 && (
        <PackageEditor
          key={editing.attempt}
          pkg={editing.pkg}
          open={editing.open}
          onOpenChange={(next) => setEditing((previous) => ({ ...previous, open: next }))}
        />
      )}

      {toggling && (
        <ConfirmDialog
          open={toggling.open}
          onOpenChange={(next) => setToggling({ ...toggling, open: next })}
          title={t(
            toggling.pkg.active
              ? "staff.packages.retireConfirm.title"
              : "staff.packages.restoreConfirm.title",
            { sku: toggling.pkg.sku },
          )}
          description={t(
            toggling.pkg.active
              ? "staff.packages.retireConfirm.body"
              : "staff.packages.restoreConfirm.body",
          )}
          confirmLabel={t(toggling.pkg.active ? "staff.packages.retire" : "staff.packages.restore")}
          danger={toggling.pkg.active}
          loading={save.isPending}
          onConfirm={() => save.mutate({ ...toggling.pkg, active: !toggling.pkg.active })}
        />
      )}
    </Pane>
  );
}

const SKU_PATTERN = /^[a-z0-9_.-]{3,64}$/;

const pad = (value: number) => String(value).padStart(2, "0");

/** A timestamp as the value of a `datetime-local` input (local time, to the minute). */
function toInput(timestamp: Timestamp | undefined): string {
  const date = toDate(timestamp);
  if (!date) return "";
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function fromInput(value: string): Timestamp | undefined {
  const date = value ? new Date(value) : undefined;
  return date && !Number.isNaN(date.getTime()) ? fromDate(date) : undefined;
}

/** Create a package (`pkg` null) or change one. Saving replaces every field of the package. */
function PackageEditor({
  pkg,
  open,
  onOpenChange,
}: {
  pkg: wallet.CoinPackage | null;
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t, i18n } = useTranslation(["wallet", "common"]);
  const queryClient = useQueryClient();
  const [sku, setSku] = useState(pkg?.sku ?? "");
  const [coins, setCoins] = useState(pkg ? pkg.coins.toString() : "");
  const [bonus, setBonus] = useState(pkg ? pkg.bonusCoins.toString() : "0");
  const [price, setPrice] = useState(pkg ? pkg.priceMinor.toString() : "");
  const [currency, setCurrency] = useState(pkg?.currency ?? "JPY");
  const [badge, setBadge] = useState<BadgeKey>(pkg ? badgeKey(pkg.badge) : "none");
  const [order, setOrder] = useState(pkg ? String(pkg.sortOrder) : "100");
  const [active, setActive] = useState(pkg?.active ?? true);
  const [from, setFrom] = useState(toInput(pkg?.availableFrom));
  const [until, setUntil] = useState(toInput(pkg?.availableUntil));
  const [submitted, setSubmitted] = useState(false);

  const parsedCoins = parseCoins(coins);
  const parsedBonus = parseCoins(bonus);
  const parsedPrice = parseCoins(price);
  const parsedOrder = parseCoins(order);
  const code = currency.trim().toUpperCase();
  const errors = {
    sku: SKU_PATTERN.test(sku.trim()) ? undefined : t("staff.packages.form.skuInvalid"),
    coins:
      parsedCoins !== undefined && parsedCoins > 0n
        ? undefined
        : t("staff.packages.form.coinsInvalid"),
    bonus: parsedBonus !== undefined ? undefined : t("staff.packages.form.bonusInvalid"),
    price:
      parsedPrice !== undefined && parsedPrice > 0n
        ? undefined
        : t("staff.packages.form.priceInvalid"),
    currency: /^[A-Z]{3}$/.test(code) ? undefined : t("staff.packages.form.currencyInvalid"),
    order:
      parsedOrder !== undefined && parsedOrder <= 1_000_000n
        ? undefined
        : t("staff.packages.form.orderInvalid"),
    until:
      from && until && new Date(until) <= new Date(from)
        ? t("staff.packages.form.untilInvalid")
        : undefined,
  };
  const valid = Object.values(errors).every((problem) => problem === undefined);
  const shown = (problem: string | undefined) => (submitted ? problem : undefined);

  const save = useMutation({
    mutationFn: () =>
      walletAdminClient().saveCoinPackage({
        package: {
          id: pkg?.id ?? "",
          sku: sku.trim(),
          coins: parsedCoins ?? 0n,
          bonusCoins: parsedBonus ?? 0n,
          priceMinor: parsedPrice ?? 0n,
          currency: code,
          badge: BADGE_KEYS[badge],
          sortOrder: Number(parsedOrder ?? 0n),
          active,
          availableFrom: fromInput(from),
          availableUntil: fromInput(until),
        },
      }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: walletKeys.admin.packages });
      void queryClient.invalidateQueries({ queryKey: walletKeys.packages });
      toast.success(t(pkg ? "staff.packages.toast.saved" : "staff.packages.toast.created"));
      onOpenChange(false);
    },
    onError: (failure) => toast.error(errorMessage(failure)),
  });

  const submit = () => {
    setSubmitted(true);
    if (valid) save.mutate();
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={pkg ? t("staff.packages.form.editTitle") : t("staff.packages.form.createTitle")}
      footer={
        <>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={save.isPending}>
            {t("common:actions.cancel")}
          </Button>
          <Button loading={save.isPending} onClick={submit}>
            {t("common:actions.save")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <TextField
          label={t("staff.packages.form.sku")}
          hint={t("staff.packages.form.skuHint")}
          value={sku}
          onChange={(event) => setSku(event.target.value)}
          error={shown(errors.sku)}
          autoCapitalize="none"
          spellCheck={false}
          autoComplete="off"
        />
        <div className="grid grid-cols-2 gap-3">
          <TextField
            label={t("staff.packages.form.coins")}
            inputMode="numeric"
            value={coins}
            onChange={(event) => setCoins(event.target.value)}
            error={shown(errors.coins)}
            autoComplete="off"
          />
          <TextField
            label={t("staff.packages.form.bonus")}
            inputMode="numeric"
            value={bonus}
            onChange={(event) => setBonus(event.target.value)}
            error={shown(errors.bonus)}
            autoComplete="off"
          />
        </div>
        <div className="grid grid-cols-[minmax(0,2fr)_minmax(0,1fr)] gap-3">
          <TextField
            label={t("staff.packages.form.price")}
            inputMode="numeric"
            value={price}
            onChange={(event) => setPrice(event.target.value)}
            error={shown(errors.price)}
            hint={
              parsedPrice !== undefined && !errors.currency
                ? t("staff.packages.form.pricePreview", {
                    price: formatMoney(parsedPrice, code, i18n.language),
                  })
                : t("staff.packages.form.priceHint")
            }
            autoComplete="off"
          />
          <TextField
            label={t("staff.packages.form.currency")}
            value={currency}
            maxLength={3}
            onChange={(event) => setCurrency(event.target.value)}
            error={shown(errors.currency)}
            autoCapitalize="characters"
            autoComplete="off"
          />
        </div>
        <div className="grid grid-cols-[minmax(0,2fr)_minmax(0,1fr)] gap-3">
          <SelectField<BadgeKey>
            label={t("staff.packages.form.badge")}
            value={badge}
            onChange={setBadge}
            options={(Object.keys(BADGE_KEYS) as BadgeKey[]).map((key) => ({
              value: key,
              label: key === "none" ? t("staff.packages.form.badgeNone") : t(`shop.badge.${key}`),
            }))}
          />
          <TextField
            label={t("staff.packages.form.order")}
            inputMode="numeric"
            value={order}
            onChange={(event) => setOrder(event.target.value)}
            error={shown(errors.order)}
            autoComplete="off"
          />
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          <TextField
            label={t("staff.packages.form.from")}
            type="datetime-local"
            value={from}
            onChange={(event) => setFrom(event.target.value)}
          />
          <TextField
            label={t("staff.packages.form.until")}
            type="datetime-local"
            value={until}
            onChange={(event) => setUntil(event.target.value)}
            error={shown(errors.until)}
          />
        </div>
        <p className="-mt-2 px-1 text-xs text-ink-faint">{t("staff.packages.form.windowHint")}</p>
        <SwitchRow
          label={t("staff.packages.form.active")}
          hint={t("staff.packages.form.activeHint")}
          checked={active}
          onChange={setActive}
        />
      </div>
    </Dialog>
  );
}
