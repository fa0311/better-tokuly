import { useFriendsSync } from "./data";

/** Runs while someone is signed in, on every screen: requests and friends stay live. */
export default function FriendsBackground() {
  useFriendsSync();
  return null;
}
