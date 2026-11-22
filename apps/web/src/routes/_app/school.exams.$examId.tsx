import { createFileRoute } from "@tanstack/react-router";
import { ExamScreen } from "#/features/school/exams-screen";

export const Route = createFileRoute("/_app/school/exams/$examId")({ component: ExamPage });

function ExamPage() {
  const { examId } = Route.useParams();
  return <ExamScreen key={examId} examId={examId} />;
}
