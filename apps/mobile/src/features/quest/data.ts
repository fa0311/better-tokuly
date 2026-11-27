import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type common, quest, wallet } from "@tokuly/api";
import { localized, questQueries } from "@tokuly/core/quest";
import { walletQueries } from "@tokuly/core/wallet";
import { useTranslation } from "react-i18next";
import { type Account, currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";

/**
 * Quests in the query cache. The keys, query options, rules and cache edits
 * are `@tokuly/core/quest`; this file wires them to the phone's API client
 * (`api()` when called: whoever is signed in now).
 */
export const questClient = () => api().service(quest.QuestService);
export const missionClient = () => api().service(quest.MissionService);
export const badgeClient = () => api().service(quest.BadgeService);
const walletClient = () => api().service(wallet.WalletService);

/**
 * The caller's coins: the wallet's own balance in the cache
 * (`walletKeys.balance`), so a bonus claimed in the wallet or a push shows
 * here too. Read again after every claim.
 */
export function useCoins() {
  const queryClient = useQueryClient();
  return useQuery({
    ...walletQueries.balance(walletClient, queryClient),
    select: (balance) => balance.coins,
  });
}

export const useProgress = () => useQuery(questQueries.progress(questClient));
export const useMissions = () => useQuery(questQueries.missions(missionClient));
export const useBadgeCatalog = () => useQuery(questQueries.catalog(badgeClient));
export const useBadge = (key: string) => useQuery(questQueries.badge(badgeClient, key));
/** A player's badges, showcase and title (`""` = the caller). */
export const useUserBadges = (userId: string) =>
  useQuery(questQueries.userBadges(badgeClient, userId));

/** `localized` bound to the language the app is shown in. */
export function useLocalized(): (text: common.LocalizedText | undefined) => string {
  const { i18n } = useTranslation();
  return (text) => localized(text, i18n.language);
}

/**
 * Sends one request as the account signed in when it was asked, through
 * that account's own client, and hands back its answer — or its refusal —
 * only while that account still is (`undefined` otherwise: whoever took
 * over sees nothing of it; README, "Accounts").
 */
export async function asAccount<T>(send: (account: Account) => Promise<T>): Promise<T | undefined> {
  const account = currentAccount();
  if (!account) return undefined;
  try {
    const answer = await send(account);
    return isActive(account) ? answer : undefined;
  } catch (error) {
    if (isActive(account)) throw error;
    return undefined;
  }
}
