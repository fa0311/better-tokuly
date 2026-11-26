import { create } from "@bufbuild/protobuf";
import { useQueryClient } from "@tanstack/react-query";
import { errorReason, isApiError, live } from "@tokuly/api";
import { compactCount } from "@tokuly/core/engagement";
import {
  type GiftAttempt,
  giftAttempt,
  liveKeys,
  newIdempotencyKey,
  parseCoins,
  sameGift,
  tierFor,
} from "@tokuly/core/live";
import { randomUUID } from "expo-crypto";
import { router } from "expo-router";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Alert,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  Text,
  TextInput,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Button } from "#/components/button";
import { currentAccount, isActive } from "#/features/session/account";
import { useMe } from "#/features/session/store";
import { webScreenHref } from "#/features/web/handoff";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { GiftBubble, tierColors } from "./chat-line";
import { coinsKey, useCoins, useGiftTiers } from "./data";
import { keepPendingGift, readPendingGift, settlePendingGift } from "./pending-gift";
import { liveChats } from "./room";

const newGiftKey = () => newIdempotencyKey(randomUUID);

/**
 * Sends a gift (super chat): pick an amount, see which of the twelve colours
 * it lands in and how long a message it may carry, look at the preview,
 * confirm, spend. Someone short of coins is sent to the wallet (on the web).
 *
 * Coins move, so an attempt keeps its idempotency key until the server
 * answers it: the gift whose answer was lost, confirmed again, is repeated
 * under that key and nothing is paid twice — also after going back, closing
 * the sheet or starting the app anew (`pending-gift.ts` keeps it for its
 * account). Only another amount or message is another gift. A gift is sent
 * as the account that confirmed it, with its API, or not at all.
 */
export function GiftSheet({
  stream,
  visible,
  onClose,
}: {
  stream: live.Stream;
  visible: boolean;
  onClose(): void;
}) {
  const { t, i18n } = useTranslation("live");
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const queryClient = useQueryClient();
  const me = useMe();
  const tiersQuery = useGiftTiers();
  const balance = useCoins();
  const [amountText, setAmountText] = useState("100");
  const [message, setMessage] = useState("");
  const [confirming, setConfirming] = useState(false);
  const [sending, setSending] = useState(false);
  /** The gift whose answer was lost, if any (see above). */
  const [lost, setLost] = useState<GiftAttempt | null>(null);

  const tiers = tiersQuery.data?.tiers ?? [];
  const minCoins = tiersQuery.data?.minCoins ?? 10n;
  const maxCoins = tiersQuery.data?.maxCoins ?? 100_000n;
  const coins = parseCoins(amountText);
  const inRange = coins !== null && coins >= minCoins && coins <= maxCoins;
  const tier = inRange ? tierFor(tiers, coins) : undefined;
  const maxLength = tier?.maxMessageLength ?? 0;
  const text = message.slice(0, maxLength).trim();
  const short = inRange && balance.data !== undefined && balance.data < coins;
  const streamId = stream.summary?.id ?? "";
  const channelName = stream.summary?.channel?.name ?? "";
  /** Confirming now would repeat the gift whose answer was lost. */
  const repeating = lost !== null && lost.coins === coins && lost.message === text;
  const userId = me?.user?.id;

  // A gift left unanswered when the sheet (or the app) was last closed comes back to be finished.
  useEffect(() => {
    if (!userId || !streamId) return;
    let closed = false;
    void readPendingGift(userId, streamId).then((pending) => {
      if (closed || !pending) return;
      setLost((current) => current ?? pending);
      setAmountText(String(pending.coins));
      setMessage(pending.message);
      setConfirming(true);
    });
    return () => {
      closed = true;
    };
  }, [userId, streamId]);
  const format = (value: bigint) => compactCount(Number(value), i18n.language);

  const back = () => setConfirming(false);
  const close = () => {
    if (sending) return;
    back();
    onClose();
  };

  const send = async () => {
    const account = currentAccount();
    if (coins === null || !account) return;
    const attempt = giftAttempt(lost, coins, text, newGiftKey);
    const client = account.api.service(live.LiveGiftService);
    setLost(attempt);
    setSending(true);
    try {
      await keepPendingGift(account.userId, streamId, attempt);
      if (!isActive(account)) return;
      const response = await client.sendGift({
        streamId,
        coins: attempt.coins,
        message: attempt.message,
        idempotencyKey: attempt.key,
      });
      await settlePendingGift(account.userId, attempt);
      if (response.message) liveChats.addOwn(queryClient, response.message);
      void queryClient.invalidateQueries({ queryKey: coinsKey });
      void queryClient.invalidateQueries({ queryKey: liveKeys.gifts });
      void queryClient.invalidateQueries({ queryKey: liveKeys.supporters(streamId) });
      // Never claim a gift that was not the one sent: an earlier attempt can come back instead.
      Alert.alert(
        response.gift && !sameGift(attempt, response.gift)
          ? t("gift.otherSent", { coins: format(response.gift.coins), name: channelName })
          : t("gift.sent", { name: channelName }),
      );
      setLost(null);
      setConfirming(false);
      setMessage("");
      onClose();
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "live" }));
      // A refusal settles the attempt (nothing was paid); only a lost answer is retried under the same key.
      if (isApiError(error) && errorReason(error)) {
        await settlePendingGift(account.userId, attempt);
        setLost(null);
        setConfirming(false);
        void queryClient.invalidateQueries({ queryKey: coinsKey });
      }
    } finally {
      setSending(false);
    }
  };

  const preview = create(live.ChatMessageSchema, {
    kind: live.ChatMessageKind.GIFT,
    author: { user: me?.user },
    text,
    gift: { coins: coins ?? 0n, tier: tier?.level ?? 0 },
  });

  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={close}>
      <Pressable accessibilityRole="none" className="flex-1 bg-black/40" onPress={close} />
      <KeyboardAvoidingView behavior={Platform.OS === "ios" ? "padding" : undefined}>
        <View
          testID="gift-sheet"
          className="max-h-[560px] rounded-t-3xl bg-surface"
          style={{ paddingBottom: insets.bottom + 12 }}
        >
          <ScrollView
            contentContainerClassName="gap-4 px-4 pt-4"
            keyboardShouldPersistTaps="handled"
          >
            <View className="gap-1">
              <Text className="text-lg font-black text-ink">
                {confirming ? t("gift.confirmTitle") : t("gift.title", { name: channelName })}
              </Text>
              <Text className="text-sm text-ink-soft">
                {confirming ? t("gift.confirmLead") : t("gift.lead")}
              </Text>
            </View>
            {confirming && repeating && !sending && (
              <Text
                accessibilityRole="alert"
                className="rounded-2xl bg-sun px-4 py-3 text-sm font-bold text-[#17122b]"
              >
                {t("gift.retryHint")}
              </Text>
            )}
            {!confirming && (
              <>
                <View className="flex-row items-center justify-between rounded-2xl bg-surface-2 px-4 py-2.5">
                  <Text className="text-sm font-bold text-ink-soft">{t("gift.balance")}</Text>
                  <Text testID="gift-balance" className="font-black text-ink">
                    {balance.data === undefined ? "…" : `🪙 ${format(balance.data)}`}
                  </Text>
                </View>
                <View className="flex-row flex-wrap gap-1.5">
                  {tiers.map((option) => {
                    const look = tierColors(option);
                    const selected = tier?.level === option.level;
                    return (
                      <Pressable
                        key={option.level}
                        testID={`gift-tier-${option.level}`}
                        accessibilityRole="button"
                        accessibilityState={{ selected }}
                        accessibilityLabel={t("gift.tierLabel", {
                          level: option.level,
                          coins: format(option.minCoins),
                        })}
                        onPress={() => setAmountText(String(option.minCoins))}
                        className={`h-10 w-[23%] items-center justify-center rounded-xl border-2 ${selected ? "border-ink" : "border-transparent"}`}
                        style={{ backgroundColor: look.background }}
                      >
                        <Text className="text-xs font-black" style={{ color: look.text }}>
                          {format(option.minCoins)}
                        </Text>
                      </Pressable>
                    );
                  })}
                </View>
                <View className="gap-1">
                  <Text className="px-1 text-sm font-bold text-ink-soft">{t("gift.amount")}</Text>
                  <TextInput
                    testID="gift-amount"
                    value={amountText}
                    onChangeText={setAmountText}
                    keyboardType="number-pad"
                    accessibilityLabel={t("gift.amount")}
                    placeholderTextColor={colors["ink-faint"]}
                    className="h-12 rounded-control border-2 border-line bg-surface px-4 text-base text-ink"
                  />
                  {amountText !== "" && !inRange ? (
                    <Text className="px-1 text-sm font-bold text-danger">
                      {t("gift.amountRange", { min: format(minCoins), max: format(maxCoins) })}
                    </Text>
                  ) : short ? (
                    <Text className="px-1 text-sm font-bold text-danger">{t("gift.short")}</Text>
                  ) : null}
                </View>
                {maxLength > 0 ? (
                  <View className="gap-1">
                    <Text className="px-1 text-sm font-bold text-ink-soft">
                      {t("gift.message")}
                    </Text>
                    <TextInput
                      testID="gift-message"
                      value={message}
                      onChangeText={setMessage}
                      maxLength={maxLength}
                      multiline
                      accessibilityLabel={t("gift.message")}
                      className="min-h-20 rounded-control border-2 border-line bg-surface px-4 py-3 text-base text-ink"
                    />
                    <Text className="px-1 text-xs text-ink-faint">
                      {t("gift.messageHint", { count: message.length, max: maxLength })}
                    </Text>
                  </View>
                ) : (
                  inRange && (
                    <Text className="px-1 text-sm text-ink-soft">{t("gift.noMessage")}</Text>
                  )
                )}
              </>
            )}
            {inRange && tier && (
              <View className="gap-2">
                <Text className="px-1 text-sm font-bold text-ink-soft">{t("gift.preview")}</Text>
                <GiftBubble message={preview} tiers={tiers} />
                {tier.highlightSeconds > 0 && (
                  <Text className="px-1 text-xs text-ink-faint">
                    {t("gift.highlightFor", { count: Math.round(tier.highlightSeconds / 60) || 1 })}
                  </Text>
                )}
              </View>
            )}
            <View className="flex-row justify-end gap-2 pb-2">
              {confirming ? (
                <>
                  <Button variant="ghost" onPress={back} disabled={sending}>
                    {t("gift.back")}
                  </Button>
                  <Button testID="gift-confirm" loading={sending} onPress={() => void send()}>
                    {t("gift.confirm", { coins: format(coins ?? 0n) })}
                  </Button>
                </>
              ) : short ? (
                <Button
                  testID="gift-buy"
                  variant="lime"
                  onPress={() => {
                    close();
                    router.push(webScreenHref("/wallet/buy") as never);
                  }}
                >
                  {t("gift.buyCoins")}
                </Button>
              ) : (
                <Button
                  testID="gift-next"
                  disabled={!inRange || !tier}
                  onPress={() => setConfirming(true)}
                >
                  {t("gift.next")}
                </Button>
              )}
            </View>
          </ScrollView>
        </View>
      </KeyboardAvoidingView>
    </Modal>
  );
}
