import { test, expect } from "@playwright/test";

// Credentials must match SERVER_ENV in playwright.config.ts.
const ADMIN_EMAIL = "admin@e2e.test";
const ADMIN_PASSWORD = "playwright-test-pw";

// ---- Login page ----

test("login page renders with correct title and form", async ({ page }) => {
  await page.goto("/login");
  await expect(page).toHaveTitle(/Login.*Stomatopod/i);
  await expect(page.locator('input[name="email"]')).toBeVisible();
  await expect(page.locator('input[name="password"]')).toBeVisible();
  await expect(page.getByRole("button", { name: /sign in/i })).toBeVisible();
});

test("invalid credentials show an error message", async ({ page }) => {
  await page.goto("/login");
  await page.fill('input[name="email"]', ADMIN_EMAIL);
  await page.fill('input[name="password"]', "definitely-wrong-password");
  await page.click('button[type="submit"]');

  await expect(page.locator(".error")).toBeVisible();
  await expect(page.locator(".error")).toContainText(/invalid credentials/i);
  // Should stay on the login page.
  await expect(page).toHaveURL(/\/login/);
});

test("successful login redirects to the dashboard", async ({ page }) => {
  await page.goto("/login");
  await page.fill('input[name="email"]', ADMIN_EMAIL);
  await page.fill('input[name="password"]', ADMIN_PASSWORD);
  await page.click('button[type="submit"]');

  // After a successful login the server redirects to /app.
  // With no sites configured the index page shows the "Your Sites" list.
  await expect(page).not.toHaveURL(/\/login/);
  await expect(page).toHaveURL(/\/app/);
  // The page should not show the error class.
  await expect(page.locator(".error")).toHaveCount(0);
});

// ---- Marketing site (public) ----

test("marketing homepage renders at / without auth", async ({ page }) => {
  await page.goto("/");
  // Should NOT redirect to /login.
  await expect(page).not.toHaveURL(/\/login/);
  await expect(page).toHaveURL(/\/$/);
  // Marketing nav has a "Sign in" CTA.
  await expect(page.getByRole("link", { name: /sign in/i })).toBeVisible();
});

// ---- Dashboard (requires auth) ----

test("unauthenticated visit to /app redirects to login", async ({ page }) => {
  await page.goto("/app");
  await expect(page).toHaveURL(/\/login/);
});

// ---- Tracker script ----

test("tracker.js is served as JavaScript", async ({ request }) => {
  const response = await request.get("/tracker.js");
  expect(response.status()).toBe(200);

  const contentType = response.headers()["content-type"] ?? "";
  expect(contentType).toContain("javascript");

  const body = await response.text();
  expect(body).toContain("function");
  expect(body).toContain("pageview");
});

test("tracker.js has long-lived cache headers", async ({ request }) => {
  const response = await request.get("/tracker.js");
  const cacheControl = response.headers()["cache-control"] ?? "";
  expect(cacheControl).toMatch(/max-age/i);
});

// ---- Analytics API auth ----

test("analytics API returns 401 without credentials", async ({ request }) => {
  const response = await request.get("/api/v1/sites");
  expect(response.status()).toBe(401);

  const body = await response.json();
  expect(body).toHaveProperty("error");
});

// ---- Ingest endpoint ----

test("ingest endpoint rejects unknown site key with 401", async ({
  request,
}) => {
  const response = await request.post("/api/v1/event", {
    data: {
      k: "unknown-site-key-that-does-not-exist",
      n: "pageview",
      u: "https://example.com/",
    },
  });
  expect(response.status()).toBe(401);
});

// ---- Login → dashboard flow (full session) ----

test("logged-in user can access the sites list page", async ({ page }) => {
  // Log in first.
  await page.goto("/login");
  await page.fill('input[name="email"]', ADMIN_EMAIL);
  await page.fill('input[name="password"]', ADMIN_PASSWORD);
  await page.click('button[type="submit"]');
  await expect(page).not.toHaveURL(/\/login/);

  // Navigate to sites; requires an authenticated session cookie.
  await page.goto("/app/sites");
  await expect(page).not.toHaveURL(/\/login/);
  // The page title comes from base.html
  await expect(page).toHaveTitle(/stomatopod/i);
});

test("logout clears session and requires re-authentication", async ({
  page,
}) => {
  // Log in.
  await page.goto("/login");
  await page.fill('input[name="email"]', ADMIN_EMAIL);
  await page.fill('input[name="password"]', ADMIN_PASSWORD);
  await page.click('button[type="submit"]');
  await expect(page).not.toHaveURL(/\/login/);

  // Logout via POST (the server expects a POST).
  await page.evaluate(async () => {
    await fetch("/logout", { method: "POST" });
  });

  // After logout the session cookie is expired; the dashboard should
  // redirect to /login. (Marketing pages at / remain public either way.)
  await page.goto("/app");
  await expect(page).toHaveURL(/\/login/);
});
