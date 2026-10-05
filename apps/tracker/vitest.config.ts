import { defineConfig } from "vitest/config";
import { fileURLToPath } from "node:url";

// Unit/integration suite against MemoryStore (Phase B). Node environment only
// — no jsdom, no fake timers: the clock is injectable everywhere (D3).

export default defineConfig({
  resolve: {
    alias: {
      "@": fileURLToPath(new URL(".", import.meta.url)),
    },
  },
  test: {
    environment: "node",
    include: ["tests/**/*.test.ts"],
    reporters: ["default"],
  },
});
