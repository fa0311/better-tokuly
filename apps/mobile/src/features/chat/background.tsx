import { useChatSync } from "./sync";

/** Runs while someone is signed in, on every screen: chat stays live. */
export default function ChatBackground() {
  useChatSync();
  return null;
}
