import { useLocalSearchParams } from "expo-router";
import { QuestionScreen } from "#/features/ask/question-screen";

export default function AskQuestion() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return <QuestionScreen questionId={id ?? ""} />;
}
