/**
 * Which video moved, and what it now comes after, given a list before and
 * after one item was dragged elsewhere. `null` when nothing moved.
 */
export function movedItem(
  before: string[],
  after: string[],
): { id: string; afterId: string } | null {
  const first = before.findIndex((id, index) => id !== after[index]);
  if (first < 0) return null;
  // Dragged down, the rest of the list slides up by one; dragged up, the item now sits right here.
  const id =
    after[first] === before[first + 1] ? (before[first] as string) : (after[first] as string);
  const position = after.indexOf(id);
  return { id, afterId: position > 0 ? (after[position - 1] as string) : "" };
}
