import { cn } from "#/lib/cn";
import { CopyButton } from "../parts";
import { highlight, type Language, type TokenType } from "./highlight";

/**
 * Token colours: each accent of the theme mixed into the ink colour, so the
 * code reads on the light and on the dark surface without a second palette.
 */
const COLORS: Record<TokenType, string> = {
  plain: "",
  punctuation: "text-ink-soft",
  comment: "italic text-ink-faint",
  keyword: "font-bold text-brand-strong",
  function: "text-brand-strong",
  string: "text-[color-mix(in_oklab,var(--mint)_52%,var(--ink))]",
  number: "text-[color-mix(in_oklab,var(--coral)_70%,var(--ink))]",
  property: "text-[color-mix(in_oklab,var(--sky)_55%,var(--ink))]",
  variable: "text-[color-mix(in_oklab,var(--pink)_72%,var(--ink))]",
  flag: "text-[color-mix(in_oklab,var(--sun)_50%,var(--ink))]",
};

/** Highlighted code as inline spans (inside a `<pre>` of the caller's). */
export function Highlighted({ code, language }: { code: string; language: Language }) {
  return (
    <code>
      {highlight(code, language).map((token, index) =>
        token.type === "plain" ? (
          token.text
        ) : (
          <span key={index} className={COLORS[token.type]}>
            {token.text}
          </span>
        ),
      )}
    </code>
  );
}

/** A block of code with a caption and a copy button. */
export function CodeBlock({
  code,
  language,
  label,
  className,
  maxHeight = "max-h-[28rem]",
}: {
  code: string;
  language: Language;
  /** Caption above the code; also names the block for assistive technology. */
  label?: string;
  className?: string;
  maxHeight?: string;
}) {
  return (
    <figure className={cn("overflow-hidden rounded-2xl border-2 border-line", className)}>
      <figcaption className="flex min-h-10 items-center gap-2 border-b-2 border-line bg-surface-2 py-1 pr-1.5 pl-3.5">
        <span className="min-w-0 flex-1 truncate text-xs font-black text-ink-soft">{label}</span>
        <CopyButton text={code} variant="ghost" className="h-8 px-2.5 text-xs" />
      </figcaption>
      <pre
        // Scrollable, so it must be reachable from the keyboard.
        // biome-ignore lint/a11y/noNoninteractiveTabindex: see above.
        tabIndex={0}
        className={cn(
          "overflow-auto bg-surface p-3.5 font-mono text-[12.5px] leading-relaxed",
          language === "text" && "break-all whitespace-pre-wrap",
          maxHeight,
        )}
      >
        <Highlighted code={code} language={language} />
      </pre>
    </figure>
  );
}
