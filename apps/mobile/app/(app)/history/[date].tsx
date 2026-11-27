import { useLocalSearchParams } from "expo-router";
import { DayScreen } from "#/features/history/day-screen";

export default function HistoryDay() {
  const { date } = useLocalSearchParams<{ date: string }>();
  return <DayScreen key={date} date={date} />;
}
