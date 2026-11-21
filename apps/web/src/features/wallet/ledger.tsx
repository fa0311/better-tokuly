import { Link } from "@tanstack/react-router";
import { toDate, toMillis, wallet } from "@tokuly/api";
import { ArrowLeftRight, CircleDollarSign, Gift, Landmark, type LucideIcon, Radio, ShoppingBag, ShoppingCart, Smile, Undo2, Wrench } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Coins } from "#/components/ui/coins";
import { Dialog } from "#/components/ui/dialog";
import { cn } from "#/lib/cn";
import { compactAge } from "#/lib/format";
import { daysAgo, groupByDay, parseMemo } from "./format";
import { Fact, StatusChip } from "./parts";

/** The icon and colour of a movement: by what it was for when we know it, by its kind otherwise. */
function entryVisual(entry: wallet.LedgerEntry): { icon: LucideIcon; tint: string } {
  const topic = entry.reason.split(".")[0];
  if (entry.kind === wallet.EntryKind.REFUND) return { icon: Undo2, tint: "bg-coral" };
  if (topic === "purchase") return { icon: ShoppingBag, tint: "bg-lime" };
  if (topic === "daily_bonus") return { icon: Gift, tint: "bg-sun" };
  if (topic === "live") return { icon: Radio, tint: "bg-pink" };
  if (topic === "stamp") return { icon: Smile, tint: "bg-sky" };
  if (topic === "payout") return { icon: Landmark, tint: "bg-mint" };
  if (entry.kind === wallet.EntryKind.ADJUSTMENT) return { icon: Wrench, tint: "bg-surface-3 text-ink" };
  if (entry.kind === wallet.EntryKind.TRANSFER) return { icon: ArrowLeftRight, tint: "bg-pink" };
  if (entry.kind === wallet.EntryKind.SPEND) return { icon: ShoppingCart, tint: "bg-sky" };
  return { icon: CircleDollarSign, tint: "bg-sun" };
}

/** i18n key of the word for an entry's kind; a transfer reads differently for its two sides. */
function kindKey(entry: wallet.LedgerEntry): string {
  switch (entry.kind) {
    case wallet.EntryKind.GRANT:
      return "kind.grant";
    case wallet.EntryKind.SPEND:
      return "kind.spend";
    case wallet.EntryKind.TRANSFER:
      return entry.coinsDelta < 0n ? "kind.sent" : "kind.received";
    case wallet.EntryKind.REFUND:
      return "kind.refund";
    case wallet.EntryKind.PAYOUT:
      return "kind.payout";
    case wallet.EntryKind.ADJUSTMENT:
      return "kind.adjustment";
    default:
      return "kind.other";
  }
}

/** A signed amount; gains sit on a lime pill so they stand out without relying on colour alone. */
function Delta({ amount }: { amount: bigint }) {
  return <Coins amount={amount} signed className={cn(amount > 0n && "rounded-full bg-lime py-0.5 pr-2 pl-1 text-[#17122b]")} />;
}

/** One movement in a history list. */
export function LedgerRow({ entry, when, onOpen }: { entry: wallet.LedgerEntry; /** Show the time of day (lists grouped by day) or how long ago. */ when: "time" | "age"; onOpen(): void }) {
  const { t, i18n } = useTranslation("wallet");
  const { icon: Icon, tint } = entryVisual(entry);
  const created = toDate(entry.createdAt);
  const time = when === "age" ? compactAge(toMillis(entry.createdAt)) : created?.toLocaleTimeString(i18n.language, { hour: "2-digit", minute: "2-digit" });

  return (
    <button type="button" onClick={onOpen} className="flex w-full items-center gap-3 rounded-2xl px-3 py-2.5 text-left transition-colors hover:bg-surface-2">
      <span className={cn("grid size-11 shrink-0 place-items-center rounded-2xl border-2 border-ink text-[#17122b]", tint)}>
        <Icon className="size-5" />
      </span>
      <span className="min-w-0 flex-1">
        <span className={cn("line-clamp-2 leading-snug font-bold break-words", entry.refunded && "text-ink-soft line-through")}>{entry.description || t(kindKey(entry))}</span>
        <span className="mt-0.5 flex items-center gap-1.5 text-sm text-ink-soft">
          {/* Who it was with when there is someone; otherwise what kind of movement it was. */}
          {entry.counterpart ? (
            <>
              <Avatar user={entry.counterpart} size="xs" />
              <span className="truncate">{entry.counterpart.displayName}</span>
            </>
          ) : (
            <span className="truncate">{t(kindKey(entry))}</span>
          )}
          <span aria-hidden>·</span>
          <span className="shrink-0">{time}</span>
        </span>
      </span>
      <span className="flex shrink-0 flex-col items-end gap-1">
        {entry.coinsDelta !== 0n && <Delta amount={entry.coinsDelta} />}
        {entry.earningsDelta !== 0n && (
          <>
            <Delta amount={entry.earningsDelta} />
            <span className="text-[11px] leading-none font-black text-ink-soft">{t("ledger.earnings")}</span>
          </>
        )}
        {entry.refunded && <StatusChip tone="neutral">{t("ledger.refunded")}</StatusChip>}
      </span>
    </button>
  );
}

/** Memo fields holding an ID are for deep links (see below), not for reading. */
const isReference = (key: string) => key === "id" || key.endsWith("_id");

/** Everything about one movement. */
function EntryDialog({ entry, open, onOpenChange, links }: { entry: wallet.LedgerEntry; open: boolean; onOpenChange(open: boolean): void; links: boolean }) {
  const { t, i18n } = useTranslation("wallet");
  const memo = parseMemo(entry.memoJson);
  const created = toDate(entry.createdAt);
  const details = Object.entries(memo).filter(([key]) => !isReference(key));
  // The platform fee comes out of what the recipient gets; the payer simply paid the amount.
  const showFee = entry.fee > 0n && entry.earningsDelta !== 0n;

  return (
    <Dialog open={open} onOpenChange={onOpenChange} title={entry.description || t(kindKey(entry))} description={t(kindKey(entry))}>
      <dl className="divide-y-2 divide-line">
        {entry.coinsDelta !== 0n && (
          <Fact label={t("entry.coins")}>
            <Coins amount={entry.coinsDelta} signed />
          </Fact>
        )}
        {entry.earningsDelta !== 0n && (
          <Fact label={t("entry.earnings")}>
            <Coins amount={entry.earningsDelta} signed />
          </Fact>
        )}
        {showFee && (
          <>
            <Fact label={t("entry.amount")}>
              <Coins amount={entry.amount} />
            </Fact>
            <Fact label={t("entry.fee")}>
              <Coins amount={entry.fee} />
            </Fact>
          </>
        )}
        {entry.counterpart && (
          <Fact label={t("entry.with")}>
            <Link to="/u/$handle" params={{ handle: entry.counterpart.handle }} className="inline-flex max-w-full items-center gap-2 underline decoration-2 underline-offset-2">
              <Avatar user={entry.counterpart} size="xs" />
              <span className="truncate">{entry.counterpart.displayName}</span>
            </Link>
          </Fact>
        )}
        {created && <Fact label={t("entry.when")}>{created.toLocaleString(i18n.language, { dateStyle: "medium", timeStyle: "short" })}</Fact>}
        {entry.refunded && <Fact label={t("entry.state")}>{t("ledger.refunded")}</Fact>}
        {details.map(([key, value]) => (
          <Fact key={key} label={i18n.exists(`wallet:memo.${key}`) ? t(`memo.${key}`) : key}>
            <span className="break-all">{value}</span>
          </Fact>
        ))}
        <Fact stack label={t("entry.id")}>
          <span className="break-all font-mono text-xs font-medium text-ink-soft">{entry.id}</span>
        </Fact>
        {entry.refundOfEntryId && (
          <Fact stack label={t("entry.refundOf")}>
            <span className="break-all font-mono text-xs font-medium text-ink-soft">{entry.refundOfEntryId}</span>
          </Fact>
        )}
      </dl>
      <div className="mt-4 flex flex-wrap gap-x-4 gap-y-2 empty:hidden">
        {links && memo.purchase_id && (
          <Link to="/wallet/purchases/$purchaseId" params={{ purchaseId: memo.purchase_id }} className="font-bold text-brand underline decoration-2 underline-offset-2">
            {t("entry.openPurchase")}
          </Link>
        )}
        {links && memo.payout_id && (
          <Link to="/wallet/payouts/$payoutId" params={{ payoutId: memo.payout_id }} className="font-bold text-brand underline decoration-2 underline-offset-2">
            {t("entry.openPayout")}
          </Link>
        )}
        {memo.pack_id && (
          <Link to="/stamps/$packId" params={{ packId: memo.pack_id }} className="font-bold text-brand underline decoration-2 underline-offset-2">
            {t("entry.openPack")}
          </Link>
        )}
      </div>
    </Dialog>
  );
}

/** "今日" / "昨日" / a date, for the headings of a history grouped by day. */
function dayLabel(date: Date | undefined, language: string, t: (key: string) => string): string {
  if (!date) return "";
  const now = new Date();
  const ago = daysAgo(date, now);
  if (ago === 0) return t("ledger.today");
  if (ago === 1) return t("ledger.yesterday");
  return date.toLocaleDateString(language, { year: date.getFullYear() === now.getFullYear() ? undefined : "numeric", month: "long", day: "numeric", weekday: "short" });
}

/**
 * A list of movements, newest first; tapping one opens its details.
 * `grouped` adds day headings (full histories); without it rows carry their
 * own age (short "recent" lists).
 */
export function LedgerList({ entries, grouped, links = true }: { entries: wallet.LedgerEntry[]; grouped?: boolean; /** Link to the viewer's own purchases and payouts (off in staff views of someone else). */ links?: boolean }) {
  const { t, i18n } = useTranslation("wallet");
  const [selected, setSelected] = useState<wallet.LedgerEntry | null>(null);
  const [open, setOpen] = useState(false);
  const show = (entry: wallet.LedgerEntry) => {
    setSelected(entry);
    setOpen(true);
  };

  return (
    <>
      {grouped ? (
        groupByDay(entries, (entry) => toDate(entry.createdAt)).map((group) => (
          <section key={group.day} aria-label={dayLabel(group.date, i18n.language, t)}>
            <h3 className="px-3 pt-4 pb-1 text-xs font-black text-ink-faint">{dayLabel(group.date, i18n.language, t)}</h3>
            <ul>
              {group.items.map((entry) => (
                <li key={entry.id}>
                  <LedgerRow entry={entry} when="time" onOpen={() => show(entry)} />
                </li>
              ))}
            </ul>
          </section>
        ))
      ) : (
        <ul>
          {entries.map((entry) => (
            <li key={entry.id}>
              <LedgerRow entry={entry} when="age" onOpen={() => show(entry)} />
            </li>
          ))}
        </ul>
      )}
      {selected && <EntryDialog entry={selected} open={open} onOpenChange={setOpen} links={links} />}
    </>
  );
}

/** Rows of an infinite list, without the duplicates that a row pushed live can cause on the next page. */
export function uniqueById<T extends { id: string }>(items: T[]): T[] {
  const seen = new Set<string>();
  return items.filter((item) => {
    if (seen.has(item.id)) return false;
    seen.add(item.id);
    return true;
  });
}
