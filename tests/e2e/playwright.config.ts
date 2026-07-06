import { defineConfig, devices } from "@playwright/test";
import * as path from "path";

// Workspace root is two directories up from this config file.
const workspaceRoot = path.resolve(__dirname, "../..");

// The server listens on a dedicated port to avoid clashing with dev instances.
const E2E_PORT = process.env.E2E_PORT ?? "18080";
const BASE_URL = `http://localhost:${E2E_PORT}`;

// Config file (relative to workspaceRoot) that the test server reads.
const E2E_CONFIG = "tests/e2e/stomatopod.toml";

// Bootstrap env vars — not in the TOML so they stay out of source control
// for real deployments; the values here are for the throwaway test instance.
const SERVER_ENV: Record<string, string> = {
  STOMATOPOD_ADMIN_EMAIL:
    process.env.STOMATOPOD_ADMIN_EMAIL ?? "admin@e2e.test",
  STOMATOPOD_ADMIN_PASSWORD:
    process.env.STOMATOPOD_ADMIN_PASSWORD ?? "playwright-test-pw",
  // Fullstack SSR + hydration needs the built wasm client bundle. Build it
  // first with `dx build --platform web` (mise's e2e task does this); override
  // for a release bundle via DIOXUS_PUBLIC_PATH.
  DIOXUS_PUBLIC_PATH:
    process.env.DIOXUS_PUBLIC_PATH ??
    path.resolve(workspaceRoot, "target/dx/stomatopod/debug/web/public"),
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
      ? `${process.env.STOMATOPOD_BIN} --config ${E2E_CONFIG} serve`
      : `cargo run --bin stomatopod -- --config ${E2E_CONFIG} serve`,
    url: `${BASE_URL}/login`,
    reuseExistingServer: !process.env.CI,
    // Allow up to 2 minutes for cargo to compile on a cold cache.
    timeout: 120_000,
    cwd: workspaceRoot,
    env: SERVER_ENV,
  },
});
