import { defineConfig } from "vitest/config";

// Unit tests run without the app's build plugins (SSR, routing, compiler).
export default defineConfig({
  resolve: { tsconfigPaths: true },
  test: { environment: "node", include: ["src/**/*.test.ts", "src/**/*.test.tsx"] },
});
