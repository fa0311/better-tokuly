import { cva, type VariantProps } from "class-variance-authority";
import type { ButtonHTMLAttributes, ReactNode } from "react";
import { cn } from "#/lib/cn";
import { Spinner } from "./spinner";

const button = cva(
  "pressable inline-flex select-none items-center justify-center gap-2 whitespace-nowrap font-bold disabled:opacity-50",
  {
    variants: {
      variant: {
        /** The one main action on a screen. */
        primary: "sticker bg-brand text-on-brand",
        /** Celebratory / positive actions. */
        lime: "sticker bg-lime text-[#17122b]",
        /** Neutral actions next to a primary one. */
        secondary: "sticker bg-surface text-ink",
        /** Low-emphasis actions inside dense UI. */
        ghost: "text-ink hover:bg-surface-2 active:bg-surface-3",
        danger: "sticker bg-danger text-white",
      },
      size: {
        sm: "h-9 rounded-full px-4 text-sm",
        md: "h-11 rounded-full px-5 text-[15px]",
        lg: "h-14 rounded-full px-7 text-lg",
        icon: "size-11 rounded-full",
        "icon-sm": "size-9 rounded-full",
      },
      block: { true: "w-full" },
    },
    defaultVariants: { variant: "primary", size: "md" },
  },
);

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement>, VariantProps<typeof button> {
  /** Shows a spinner and blocks clicks while an action is in flight. */
  loading?: boolean;
  icon?: ReactNode;
}

export function Button({ className, variant, size, block, loading, icon, children, disabled, type, ...props }: ButtonProps) {
  return (
    <button
      type={type ?? "button"}
      className={cn(button({ variant, size, block }), className)}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      {...props}
    >
      {loading ? <Spinner className="size-4" /> : icon}
      {children}
    </button>
  );
}
