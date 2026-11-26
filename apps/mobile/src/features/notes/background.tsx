import { useNotesSync } from "./live";

/** Runs while someone is signed in, on every screen: notes stay live. */
export default function NotesBackground() {
  useNotesSync();
  return null;
}
