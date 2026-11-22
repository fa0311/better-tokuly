import { type QueryClient, useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import type { ask } from "@tokuly/api";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";
import { addToInbox, askKeys, rememberCounts } from "./api";
import { offerDraft, takePendingAsk } from "./guest-return";

/**
 * Applies a realtime `AskEvent` to what the app holds. Nothing is announced
 * here: each of these also arrives as a notification, which has its toast.
 */
export function applyAskEvent(queryClient: QueryClient, event: ask.AskEvent["event"]): void {
  switch (event.case) {
    case "questionReceived": {
      rememberCounts(queryClient, event.value.counts);
      if (event.value.question) addToInbox(queryClient, event.value.question);
      break;
    }
    case "questionAnswered": {
      const question = event.value.question;
      if (question) queryClient.setQueryData(askKeys.question(question.id), question);
      void queryClient.invalidateQueries({ queryKey: askKeys.asked });
      break;
    }
    case "flameReceived": {
      const unseen = event.value.unseen;
      queryClient.setQueryData<ask.FlameSummary>(askKeys.flameSummary, (known) => (known ? { ...known, unseen } : known));
      void queryClient.invalidateQueries({ queryKey: askKeys.flameSummary });
      void queryClient.invalidateQueries({ queryKey: askKeys.flames });
      void queryClient.invalidateQueries({ queryKey: askKeys.topFlames });
      break;
    }
    case "promptResponseReceived": {
      const promptId = event.value.prompt?.id ?? event.value.response?.promptId;
      if (!promptId) break;
      void queryClient.invalidateQueries({ queryKey: askKeys.prompt(promptId) });
      void queryClient.invalidateQueries({ queryKey: askKeys.responsesAll(promptId) });
      void queryClient.invalidateQueries({ queryKey: askKeys.promptsAll });
      break;
    }
    default:
      break;
  }
}

/**
 * The question box, for as long as someone is signed in: new questions,
 * answers, flames and prompt responses pushed over the realtime connection
 * go into the query cache, so the inbox, the badges and "your question was
 * answered" are live on every page. It also finishes what a visitor started
 * on a public box page before they had an account.
 */
export default function AskBackground() {
  const queryClient = useQueryClient();
  const router = useRouter();

  useEffect(() => {
    const client = realtime();
    const offFrame = client.onFrame((frame) => {
      if (frame.body.case === "ask") applyAskEvent(queryClient, frame.body.value.event);
      // Pushes sent while there was no connection (or before this one was up) are lost: read again what is on screen.
      else if (frame.body.case === "welcome") void queryClient.invalidateQueries({ queryKey: askKeys.all });
    });
    client.start();
    return offFrame;
  }, [queryClient]);

  // Someone who wrote a question on `/q/<handle>` and then signed up (or in) goes back to that box.
  useEffect(() => {
    const pending = takePendingAsk();
    if (!pending) return;
    offerDraft(pending);
    void router.navigate({ to: "/ask/u/$handle", params: { handle: pending.handle }, replace: true });
  }, [router]);

  return null;
}
