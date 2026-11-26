import { ask, common } from "@tokuly/api";
import type { Subject } from "../engagement";

/** Limits of the API, for counters and `maxLength`. */
export const QUESTION_MAX = 300;
export const ANSWER_MAX = 1000;

/** Length as the server counts it: characters, not UTF-16 units (an emoji is one). */
export const charCount = (text: string): number => Array.from(text).length;

/** A question (with its answer) as the subject of likes, comments and reports. */
export const questionSubject = (id: string): Subject => ({ type: common.SubjectType.QUESTION, id });

/** Whether likes and comments exist for this answer: only on the Q&A page (public or friends). */
export const hasEngagement = (question: ask.Question): boolean =>
  question.answer?.audience === ask.AnswerAudience.PUBLIC ||
  question.answer?.audience === ask.AnswerAudience.FRIENDS;

/** The public page of a box on `origin`: what a share link and the image card point at. */
export function boxShareUrl(handle: string, origin: string): string {
  return `${origin}/q/${handle}`;
}

/** The public page of a box with one answer brought to the top. */
export function answerShareUrl(handle: string, questionId: string, origin: string): string {
  return `${boxShareUrl(handle, origin)}?a=${questionId}`;
}
