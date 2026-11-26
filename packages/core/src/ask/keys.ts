import type { ask } from "@tokuly/api";

/**
 * Query keys of the ask domain (question boxes, prompts, compliment polls).
 * Everything starts with `"ask"`. A key names the viewer's own things or an
 * item by its ID, never who sent something.
 */
export const askKeys = {
  all: ["ask"] as const,
  settings: ["ask", "settings"] as const,
  box: (user: string) => ["ask", "box", user] as const,
  counts: ["ask", "counts"] as const,
  inboxAll: ["ask", "inbox"] as const,
  inbox: (folder: ask.QuestionFolder) => ["ask", "inbox", folder] as const,
  asked: ["ask", "asked"] as const,
  answersAll: ["ask", "answers"] as const,
  answers: (user: string) => ["ask", "answers", user] as const,
  question: (id: string) => ["ask", "question", id] as const,
  blocks: ["ask", "blocks"] as const,
  promptsAll: ["ask", "prompts"] as const,
  promptsOf: (user: string) => ["ask", "prompts", "of", user] as const,
  friendPrompts: ["ask", "prompts", "friends"] as const,
  prompt: (id: string) => ["ask", "prompt", id] as const,
  responsesAll: (promptId: string) => ["ask", "responses", promptId] as const,
  responses: (promptId: string, filter: ask.ResponseFilter) =>
    ["ask", "responses", promptId, filter] as const,
  round: ["ask", "round"] as const,
  flames: ["ask", "flames"] as const,
  flameSummary: ["ask", "flameSummary"] as const,
  topFlames: ["ask", "topFlames"] as const,
};
