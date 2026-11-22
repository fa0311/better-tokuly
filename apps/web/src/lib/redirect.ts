/**
 * Where to go after signing in or signing up: the `redirect` search parameter
 * of `/login` and `/signup`, kept only when it is a path inside this app.
 * (`//host` and `/\host` would leave it, so they are refused.)
 */
export function parseRedirect(value: unknown): string | undefined {
  if (typeof value !== "string" || !value.startsWith("/")) return undefined;
  if (value.startsWith("//") || value.startsWith("/\\")) return undefined;
  return value;
}
