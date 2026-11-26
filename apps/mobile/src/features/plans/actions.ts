import type { Client } from "@connectrpc/connect";
import { plans } from "@tokuly/api";
import { Alert } from "react-native";
import { AccountChanged, currentAccount, isActive } from "#/features/session/account";
import { errorMessage } from "#/lib/errors";

type PlanService = Client<typeof plans.PlanService>;

/**
 * Sends one request about a plan as the account in use when it was asked,
 * and hands back the answer only while that account still is (`null`
 * otherwise: whoever took over sees nothing of it).
 */
export async function asViewer<R>(
  request: (service: PlanService) => Promise<R>,
): Promise<R | null> {
  const account = currentAccount();
  if (!account) return null;
  const answer = await request(account.api.service(plans.PlanService));
  return isActive(account) ? answer : null;
}

/** The server's refusal in the app's words; nothing for work an account switch has overtaken. */
export function showPlanError(error: unknown): void {
  if (error instanceof AccountChanged) return;
  Alert.alert(errorMessage(error, { namespace: "plans" }));
}
