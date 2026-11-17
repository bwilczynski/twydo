import { defineConfig, devices } from "@playwright/test";

// Phone-size WebKit against a real `tdo daemon` and `tdo gateway serve` (see e2e/setup.ts).
// One worker and no parallelism: every spec shares the one daemon and its synthetic vdir.
//
// WebKit is the default and what CI runs. On a host where WebKit cannot launch, opt in to Chromium:
//   TWYDO_E2E_BROWSER=chromium [TWYDO_E2E_CHROMIUM=/usr/bin/chromium] npm run e2e
const { defaultBrowserType: _webkit, ...device } = devices["iPhone 15"];
const chromium = process.env.TWYDO_E2E_BROWSER === "chromium";
const executablePath = process.env.TWYDO_E2E_CHROMIUM;

export default defineConfig({
  testDir: "e2e",
  testMatch: "*.spec.ts",
  globalSetup: "./e2e/setup.ts",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 30_000,
  expect: { timeout: 10_000 },
  reporter: [["list"]],
  use: {
    ...device,
    browserName: chromium ? "chromium" : "webkit",
    ...(chromium && executablePath
      ? { launchOptions: { executablePath, args: ["--no-sandbox"] } }
      : {}),
    trace: "retain-on-failure",
  },
  projects: [
    { name: "light", use: { colorScheme: "light" } },
    { name: "dark", use: { colorScheme: "dark" } },
  ],
});
