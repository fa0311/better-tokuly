import { useLocalSearchParams } from "expo-router";
import { AnswerScreen } from "#/features/ask/answer-screen";

export default function AskAnswer() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return <AnswerScreen questionId={id ?? ""} />;
}
