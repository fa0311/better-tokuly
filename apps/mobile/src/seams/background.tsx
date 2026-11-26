import backgrounds from "#/generated/seams/background";

/**
 * Background features: what runs on every screen while someone is signed in
 * (realtime listeners that keep the cache current, sharing, uploads going on).
 *
 * A feature default-exports a component, usually rendering nothing, from
 * `src/features/<area>/background.tsx`:
 *
 *   export default function ChatBackground() {
 *     useChatSync();
 *     return null;
 *   }
 *
 * It is mounted inside the account's scope: a switch or a sign-out unmounts
 * it and mounts a new one for the next account, with an empty query cache.
 */
export function BackgroundFeatures() {
  return backgrounds.map((Background, index) => <Background key={index} />);
}
