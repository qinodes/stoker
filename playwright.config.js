import path from "node:path";
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./web/tests/browser",
  timeout: 30_000,
  fullyParallel: false,
  use: {
    baseURL: "http://127.0.0.1:4173",
    browserName: "chromium",
    trace: "retain-on-failure",
  },
  webServer: {
    command: "cargo run --quiet -- ui-run --host 127.0.0.1 --port 4173",
    url: "http://127.0.0.1:4173/api/v1/ui/config",
    timeout: 120_000,
    reuseExistingServer: false,
    env: {
      ...process.env,
      STOKER_HOME: path.resolve("target/playwright-stoker-home"),
    },
  },
});
