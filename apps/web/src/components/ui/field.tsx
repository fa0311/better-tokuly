import { type InputHTMLAttributes, type ReactNode, type TextareaHTMLAttributes, useId } from "react";
import { cn } from "#/lib/cn";

const control =
  "w-full rounded-control border-2 border-line bg-surface px-4 text-[15px] text-ink placeholder:text-ink-faint " +
  "transition-colors focus:border-brand focus:outline-none disabled:opacity-60 aria-invalid:border-danger";

interface FieldShellProps {
  label?: ReactNode;
  /** Help text under the control. */
  hint?: ReactNode;
  /** Error text; also marks the control invalid. */
  error?: ReactNode;
  className?: string;
}

function FieldShell({ label, hint, error, className, id, children }: FieldShellProps & { id: string; children: ReactNode }) {
  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      {label && (
        <label htmlFor={id} className="px-1 text-sm font-bold text-ink-soft">
          {label}
        </label>
      )}
      {children}
      {error ? (
        <p id={`${id}-error`} role="alert" className="px-1 text-sm font-medium text-danger">
          {error}
        </p>
      ) : (
        hint && (
          <p id={`${id}-hint`} className="px-1 text-xs text-ink-faint">
            {hint}
          </p>
        )
      )}
    </div>
  );
}

export interface TextFieldProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "className">, FieldShellProps {
  /** Element rendered inside the control on the right (e.g. a show-password toggle). */
  trailing?: ReactNode;
}

export function TextField({ label, hint, error, className, trailing, id, ...props }: TextFieldProps) {
  const generated = useId();
  const fieldId = id ?? generated;
  return (
    <FieldShell label={label} hint={hint} error={error} className={className} id={fieldId}>
      <div className="relative">
        <input
          id={fieldId}
          className={cn(control, "h-12", trailing && "pr-12")}
          aria-invalid={error ? true : undefined}
          aria-describedby={error ? `${fieldId}-error` : hint ? `${fieldId}-hint` : undefined}
          {...props}
        />
        {trailing && <div className="absolute inset-y-0 right-1.5 flex items-center">{trailing}</div>}
      </div>
    </FieldShell>
  );
}

export interface TextAreaProps extends Omit<TextareaHTMLAttributes<HTMLTextAreaElement>, "className">, FieldShellProps {}

export function TextArea({ label, hint, error, className, id, ...props }: TextAreaProps) {
  const generated = useId();
  const fieldId = id ?? generated;
  return (
    <FieldShell label={label} hint={hint} error={error} className={className} id={fieldId}>
      <textarea
        id={fieldId}
        className={cn(control, "min-h-24 resize-y py-3 leading-relaxed")}
        aria-invalid={error ? true : undefined}
        aria-describedby={error ? `${fieldId}-error` : hint ? `${fieldId}-hint` : undefined}
        {...props}
      />
    </FieldShell>
  );
}
