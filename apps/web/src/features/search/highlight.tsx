import type { search } from "@tokuly/api";

/**
 * Text the API returned as matching and non-matching runs. Matches are marked;
 * the runs are plain text, never markup.
 */
export function Segments({ segments, fallback, className }: { segments: search.TextSegment[]; /** Shown when there are no runs (older documents, typeahead). */ fallback?: string; className?: string }) {
  if (segments.length === 0) return fallback ? <span className={className}>{fallback}</span> : null;
  return (
    <span className={className}>
      {segments.map((segment, index) =>
        segment.highlighted ? (
          <mark key={index} className="rounded-[0.3em] bg-lime px-[0.1em] text-[#17122b]">
            {segment.text}
          </mark>
        ) : (
          <span key={index}>{segment.text}</span>
        ),
      )}
    </span>
  );
}
