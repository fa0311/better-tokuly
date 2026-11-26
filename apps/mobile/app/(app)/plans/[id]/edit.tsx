import { useLocalSearchParams } from "expo-router";
import { PlanEditScreen } from "#/features/plans/plan-edit-screen";

export default function EditPlan() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return <PlanEditScreen planId={id ?? ""} />;
}
