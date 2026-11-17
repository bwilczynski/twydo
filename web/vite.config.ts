import { defineConfig } from "vitest/config";
import preact from "@preact/preset-vite";

export default defineConfig({
  plugins: [preact()],
  test: { environment: "jsdom", exclude: ["e2e/**", "node_modules/**"] },
});
