import { useLocalSearchParams } from "expo-router";
import { PlanScreen } from "#/features/plans/plan-screen";

export default function Plan() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return <PlanScreen planId={id ?? ""} />;
}
