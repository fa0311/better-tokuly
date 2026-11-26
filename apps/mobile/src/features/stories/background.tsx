import { useStoriesSync } from "./data";

/** Runs while someone is signed in, on every screen: the story tray stays live. */
export default function StoriesBackground() {
  useStoriesSync();
  return null;
}
