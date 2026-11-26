import { useLocalSearchParams } from "expo-router";
import { PlanNewScreen } from "#/features/plans/plan-new-screen";

export default function NewPlan() {
  const { invite } = useLocalSearchParams<{ invite?: string }>();
  return <PlanNewScreen inviteId={invite} />;
}
