/** Whether a new newest timeline entry is the viewer's own note. */
export function shouldScrollToNewOwnEntry(
  previousEntry: string | undefined,
  nextEntry: string | undefined,
  nextAuthorId: string | undefined,
  myId: string,
): boolean {
  return (
    previousEntry !== undefined &&
    nextEntry !== undefined &&
    previousEntry !== nextEntry &&
    nextAuthorId === myId
  );
}
