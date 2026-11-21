/**
 * Readers for values from the address bar. The router hands search parameters
 * over as parsed JSON, so an ID or a query made of digits arrives as a number.
 */

/** A non-empty text parameter, else `undefined`. */
export function parseText(input: unknown): string | undefined {
  if (typeof input === "number") return String(input);
  return typeof input === "string" && input !== "" ? input : undefined;
}

/** A parameter holding one of the allowed enum numbers, else `undefined`. */
export function parseEnum<T extends number>(input: unknown, allowed: readonly T[]): T | undefined {
  const value = typeof input === "number" ? input : typeof input === "string" && input !== "" ? Number(input) : Number.NaN;
  return allowed.find((candidate) => candidate === value);
}

/** A parameter holding one of the allowed words, else `undefined`. */
export function parseChoice<T extends string>(input: unknown, allowed: readonly T[]): T | undefined {
  return allowed.find((candidate) => candidate === input);
}
