import { defineConfig, devices } from "@playwright/test";
import * as path from "path";

// Workspace root is two directories up from this config file.
const workspaceRoot = path.resolve(__dirname, "../..");

// The server listens on a dedicated port to avoid clashing with dev instances.
const E2E_PORT = process.env.E2E_PORT ?? "18080";
const BASE_URL = `http://localhost:${E2E_PORT}`;

// Credentials injected into the server process for first-boot bootstrap.
const SERVER_ENV = {
  STOMATOPOD_AUTH__SECRET_KEY: "playwright-e2e-secret-key",
  STOMATOPOD_LISTEN__PORT: E2E_PORT,
  STOMATOPOD_ADMIN_EMAIL: "admin@e2e.test",
  STOMATOPOD_ADMIN_PASSWORD: "playwright-test-pw",
};

export default defineConfig({
  testDir: "./tests",
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: BASE_URL,
    trace: "on-first-retry",
    screenshot: "only-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    // Use a pre-built binary (STOMATOPOD_BIN) in CI; fall back to cargo run locally.
    command: process.env.STOMATOPOD_BIN
      ? `${process.env.STOMATOPOD_BIN} serve`
      : "cargo run --bin stomatopod -- serve",
    url: `${BASE_URL}/login`,
    reuseExistingServer: !process.env.CI,
    // Allow up to 2 minutes for cargo to compile on a cold cache.
    timeout: 120_000,
    cwd: workspaceRoot,
    env: SERVER_ENV,
  },
});
