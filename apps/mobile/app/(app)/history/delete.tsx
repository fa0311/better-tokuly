import { useLocalSearchParams } from "expo-router";
import { DeleteHistoryScreen } from "#/features/history/delete-screen";

export default function DeleteHistory() {
  const { date } = useLocalSearchParams<{ date?: string }>();
  return <DeleteHistoryScreen date={date || undefined} />;
}
