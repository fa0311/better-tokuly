export const EDITOR_TABS = ["details", "subtitles", "analytics"] as const;
export type EditorTab = (typeof EDITOR_TABS)[number];
