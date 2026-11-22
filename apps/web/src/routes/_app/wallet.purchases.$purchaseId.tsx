import { createFileRoute } from "@tanstack/react-router";
import { PurchaseScreen } from "#/features/wallet/purchases-screen";

export const Route = createFileRoute("/_app/wallet/purchases/$purchaseId")({
  // A payment provider's return URL may carry its proof of payment as `?proof=…`.
  validateSearch: (search: Record<string, unknown>): { proof?: string } =>
    typeof search.proof === "string" && search.proof ? { proof: search.proof } : {},
  component: PurchasePage,
});

function PurchasePage() {
  const { purchaseId } = Route.useParams();
  const { proof } = Route.useSearch();
  return <PurchaseScreen key={purchaseId} purchaseId={purchaseId} proof={proof} />;
}
