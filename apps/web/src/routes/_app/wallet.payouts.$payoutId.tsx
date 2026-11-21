import { createFileRoute } from "@tanstack/react-router";
import { PayoutScreen } from "#/features/wallet/payout-screen";

export const Route = createFileRoute("/_app/wallet/payouts/$payoutId")({ component: PayoutPage });

function PayoutPage() {
  const { payoutId } = Route.useParams();
  return <PayoutScreen key={payoutId} payoutId={payoutId} />;
}
