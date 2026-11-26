import { useLocalSearchParams } from "expo-router";
import { PlanNewScreen } from "#/features/plans/plan-new-screen";

export default function NewPlan() {
  const { invite, handle } = useLocalSearchParams<{ invite?: string; handle?: string }>();
  return <PlanNewScreen inviteId={invite} inviteHandle={handle} />;
}
