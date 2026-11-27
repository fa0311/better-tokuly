import { toDate, toMillis, wallet } from "@tokuly/api";
import { compactAge } from "@tokuly/core/format";
import { formatCoins, parseMemo } from "@tokuly/core/wallet";
import { router } from "expo-router";
import {
  ArrowLeftRight,
  CircleDollarSign,
  Gift,
  Landmark,
  type LucideIcon,
  Radio,
  ShoppingBag,
  ShoppingCart,
  Smile,
  Undo2,
  Wrench,
} from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { ActionSheet } from "#/components/action-sheet";
import { Avatar } from "#/components/avatar";
import { type ColorName, INK_ON_BRIGHT, useTheme } from "#/lib/theme";

/** A coin amount as the app writes it: the coin, then the number (`signed`: with + or −). */
export function coinText(amount: bigint, signed = false): string {
  const sign = signed ? (amount > 0n ? "+" : amount < 0n ? "−" : "") : "";
  return `🪙 ${sign}${formatCoins(amount < 0n ? -amount : amount)}`;
}

/** The icon and tint of a movement: by what it was for when known, by its kind otherwise (the web's choice). */
function entryVisual(entry: wallet.LedgerEntry): { Icon: LucideIcon; tint: ColorName } {
  const topic = entry.reason.split(".")[0];
  if (entry.kind === wallet.EntryKind.REFUND) return { Icon: Undo2, tint: "coral" };
  if (topic === "purchase") return { Icon: ShoppingBag, tint: "lime" };
  if (topic === "daily_bonus") return { Icon: Gift, tint: "sun" };
  if (topic === "live") return { Icon: Radio, tint: "pink" };
  if (topic === "stamp") return { Icon: Smile, tint: "sky" };
  if (topic === "payout") return { Icon: Landmark, tint: "mint" };
  if (entry.kind === wallet.EntryKind.ADJUSTMENT) return { Icon: Wrench, tint: "surface-2" };
  if (entry.kind === wallet.EntryKind.TRANSFER) return { Icon: ArrowLeftRight, tint: "pink" };
  if (entry.kind === wallet.EntryKind.SPEND) return { Icon: ShoppingCart, tint: "sky" };
  return { Icon: CircleDollarSign, tint: "sun" };
}

/** `wallet:` key of the word for an entry's kind; a transfer reads differently for its two sides. */
export function kindKey(entry: wallet.LedgerEntry): string {
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

/** A signed change; gains sit on a lime pill so they stand out without relying on colour alone. */
function Delta({ amount, testID }: { amount: bigint; testID?: string }) {
  const gain = amount > 0n;
  return (
    <Text
      testID={testID}
      className={`text-sm font-black ${gain ? "overflow-hidden rounded-full bg-lime px-2 py-0.5" : "text-ink"}`}
      style={gain ? { color: INK_ON_BRIGHT } : undefined}
    >
      {coinText(amount, true)}
    </Text>
  );
}

/** One movement of a history list: what it was, with whom or what kind, when, and what it changed. */
export function EntryRow({
  entry,
  when,
  onPress,
}: {
  entry: wallet.LedgerEntry;
  /** The time of day (lists grouped by day) or how long ago. */
  when: "time" | "age";
  onPress(): void;
}) {
  const { t, i18n } = useTranslation("wallet");
  const { colors } = useTheme();
  const { Icon, tint } = entryVisual(entry);
  const time =
    when === "age"
      ? compactAge(i18n, toMillis(entry.createdAt))
      : toDate(entry.createdAt)?.toLocaleTimeString(i18n.language, {
          hour: "2-digit",
          minute: "2-digit",
        });
  return (
    <Pressable
      testID={`wallet-entry-${entry.id}`}
      accessibilityRole="button"
      onPress={onPress}
      className="min-h-16 flex-row items-center gap-3 border-b border-line px-4 py-3 active:bg-surface-2"
    >
      <View
        className="h-11 w-11 items-center justify-center rounded-2xl border-2 border-ink"
        style={{ backgroundColor: colors[tint] }}
      >
        <Icon size={20} color={tint === "surface-2" ? colors.ink : INK_ON_BRIGHT} />
      </View>
      <View className="flex-1 gap-0.5">
        <Text
          numberOfLines={2}
          className={`font-bold ${entry.refunded ? "text-ink-soft line-through" : "text-ink"}`}
        >
          {entry.description || t(kindKey(entry))}
        </Text>
        <Text numberOfLines={1} className="text-sm text-ink-soft">
          {entry.counterpart ? entry.counterpart.displayName : t(kindKey(entry))} · {time}
        </Text>
      </View>
      <View className="items-end gap-1">
        {entry.coinsDelta !== 0n && <Delta amount={entry.coinsDelta} />}
        {entry.earningsDelta !== 0n && (
          <>
            <Delta amount={entry.earningsDelta} />
            <Text className="text-[11px] font-black text-ink-soft">{t("ledger.earnings")}</Text>
          </>
        )}
        {entry.refunded && (
          <Text className="text-[11px] font-black text-ink-soft">{t("ledger.refunded")}</Text>
        )}
      </View>
    </Pressable>
  );
}

/** Memo fields holding an ID are for links, not for reading. */
const isReference = (key: string) => key === "id" || key.endsWith("_id");

/** Everything about one movement, rising from the bottom; the person it was with is one tap away. */
export function EntrySheet({
  entry,
  onClose,
}: {
  entry: wallet.LedgerEntry | null;
  onClose(): void;
}) {
  const { t, i18n } = useTranslation(["wallet", "common"]);
  if (!entry) return null;
  const created = toDate(entry.createdAt);
  const memo = Object.entries(parseMemo(entry.memoJson)).filter(([key]) => !isReference(key));
  // The platform fee comes out of what the recipient gets; the payer simply paid the amount.
  const showFee = entry.fee > 0n && entry.earningsDelta !== 0n;
  const counterpart = entry.counterpart;
  const facts: [string, string][] = [
    ...(entry.coinsDelta !== 0n
      ? [[t("entry.coins"), coinText(entry.coinsDelta, true)] as [string, string]]
      : []),
    ...(entry.earningsDelta !== 0n
      ? [[t("entry.earnings"), coinText(entry.earningsDelta, true)] as [string, string]]
      : []),
    ...(showFee
      ? [
          [t("entry.amount"), coinText(entry.amount)] as [string, string],
          [t("entry.fee"), coinText(-entry.fee, true)] as [string, string],
        ]
      : []),
    ...memo.map(([key, value]): [string, string] => [
      t(`memo.${key}`, { defaultValue: key }),
      value,
    ]),
    ...(created
      ? [[t("entry.when"), created.toLocaleString(i18n.language)] as [string, string]]
      : []),
    ...(entry.refunded ? [[t("entry.state"), t("ledger.refunded")] as [string, string]] : []),
    [t("entry.id"), entry.id],
  ];
  return (
    <ActionSheet
      visible
      title={entry.description || t(kindKey(entry))}
      onClose={onClose}
      actions={[
        ...(counterpart?.handle
          ? [
              {
                key: "wallet-entry-person",
                label: counterpart.displayName,
                onPress: () => router.push(`/u/${counterpart.handle}`),
              },
            ]
          : []),
        { key: "wallet-entry-close", label: t("common:actions.close"), onPress: () => {} },
      ]}
    >
      <View testID="wallet-entry-sheet" className="mb-2 rounded-2xl bg-surface-2 px-4 py-1">
        {counterpart && (
          <View className="flex-row items-center gap-3 border-b border-line py-3">
            <Avatar user={counterpart} size="sm" />
            <Text className="flex-1 text-sm text-ink-soft">{t("entry.with")}</Text>
            <Text className="font-bold text-ink">{counterpart.displayName}</Text>
          </View>
        )}
        {facts.map(([label, value]) => (
          <View key={label} className="flex-row items-center gap-3 border-b border-line py-3">
            <Text className="text-sm text-ink-soft">{label}</Text>
            <Text selectable className="flex-1 text-right font-bold text-ink">
              {value}
            </Text>
          </View>
        ))}
      </View>
    </ActionSheet>
  );
}
