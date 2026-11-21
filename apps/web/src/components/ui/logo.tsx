import { cn } from "#/lib/cn";

/**
 * The Tokuly mark: a map pin with a face-less round "now" dot, sitting on a
 * lime disc — a nod to the original logo (pin on a yellow-green disc).
 */
export function LogoMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 64 64" className={cn("size-10", className)} role="img" aria-label="Tokuly">
      <circle cx="32" cy="34" r="26" fill="#c6f24e" stroke="#17122b" strokeWidth="3" />
      <path
        d="M32 8c-9.4 0-17 7.4-17 16.6 0 11.7 13.6 24.300 15.800 26.200a1.800 1.800 0 0 0 2.400 0C35.400 48.900 49 36.300 49 24.600 49 15.400 41.400 8 32 8Z"
        fill="#7c5cff"
        stroke="#17122b"
        strokeWidth="3"
        strokeLinejoin="round"
      />
      <circle cx="32" cy="24.500" r="6.500" fill="#ffffff" stroke="#17122b" strokeWidth="3" />
      <circle cx="46.500" cy="45.500" r="5" fill="#ff5c9d" stroke="#17122b" strokeWidth="3" />
    </svg>
  );
}

export function Logo({ className }: { className?: string }) {
  return (
    <span className={cn("inline-flex items-center gap-2 text-2xl font-black tracking-tight", className)}>
      <LogoMark className="size-9" />
      Tokuly
    </span>
  );
}
