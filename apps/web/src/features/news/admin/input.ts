/** A source carries at most this many default tags. */
export const MAX_TAGS = 10;

/** Default tags as typed: separated by commas, 読点 or spaces; '#' is optional. */
export function parseTags(text: string): string[] {
  return [
    ...new Set(
      text
        .split(/[,、\s]+/)
        .map((tag) =>
          tag
            .trim()
            .replace(/^[#＃]+/, "")
            .toLowerCase(),
        )
        .filter(Boolean),
    ),
  ];
}

const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;

/** The article ID in what staff pasted: the bare ID, or the address of its page in the app. */
export function articleIdFrom(input: string): string | undefined {
  return input.match(UUID)?.[0].toLowerCase();
}
