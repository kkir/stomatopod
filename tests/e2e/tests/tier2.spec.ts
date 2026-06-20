import { test, expect, type Page } from "@playwright/test";

// Credentials must match SERVER_ENV in playwright.config.ts.
const ADMIN_EMAIL = "admin@e2e.test";
const ADMIN_PASSWORD = "playwright-test-pw";

async function login(page: Page) {
  await page.goto("/login");
  await page.fill('input[name="email"]', ADMIN_EMAIL);
  await page.fill('input[name="password"]', ADMIN_PASSWORD);
  await page.click('button[type="submit"]');
  await expect(page).not.toHaveURL(/\/login/);
}

// Ensure at least one site exists and return its id. Uses the authenticated
// session cookie established by `login` (the analytics API accepts it).
async function ensureSite(page: Page): Promise<string> {
  let res = await page.request.get("/api/v1/sites");
  let sites = (await res.json()).sites ?? [];
  if (sites.length === 0) {
    await page.request.post("/app/sites", {
      form: { domain: "e2e-tier2.test", name: "E2E Tier2" },
    });
    res = await page.request.get("/api/v1/sites");
    sites = (await res.json()).sites ?? [];
  }
  expect(sites.length).toBeGreaterThan(0);
  return sites[0].id as string;
}

// ---- Sidebar navigation ----

test("sidebar exposes Real-time, Goals and Alerts", async ({ page }) => {
  await login(page);
  await page.goto("/app/sites");
  const nav = page.locator(".side-nav");
  await expect(nav.getByRole("link", { name: "Real-time" })).toBeVisible();
  await expect(nav.getByRole("link", { name: "Goals" })).toBeVisible();
  await expect(nav.getByRole("link", { name: "Alerts" })).toBeVisible();
});

// ---- Overview: entry/exit + export ----

test("overview shows entry/exit panels and export buttons", async ({ page }) => {
  await login(page);
  const siteId = await ensureSite(page);

  await page.goto(`/app/sites/${siteId}`);
  await expect(page.getByRole("heading", { name: "Entry Pages" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Exit Pages" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Export" })).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Events CSV" }),
  ).toBeVisible();
  // The Top Pages panel carries a CSV download link.
  await expect(page.locator('a.csv-btn').first()).toBeVisible();
});

test("top-pages CSV export downloads as text/csv", async ({ page }) => {
  await login(page);
  const siteId = await ensureSite(page);

  const res = await page.request.get(
    `/api/v1/sites/${siteId}/top-pages?range=30d&format=csv`,
  );
  expect(res.status()).toBe(200);
  expect(res.headers()["content-type"] ?? "").toContain("csv");
  expect(res.headers()["content-disposition"] ?? "").toContain("top-pages.csv");
  expect(await res.text()).toContain("value,pageviews,sessions,pct");
});

// ---- Real-time ----

test("real-time page renders with a site selector and live panel", async ({
  page,
}) => {
  await login(page);
  await ensureSite(page);

  await page.goto("/app/realtime");
  await expect(page).toHaveURL(/\/app\/realtime/);
  await expect(page.locator('select[name="site"]')).toBeVisible();
  await expect(page.getByRole("link", { name: "Real-time" }).first()).toBeVisible();
  // The panel is swapped in by htmx on load.
  await expect(page.getByText("Active Sessions", { exact: true })).toBeVisible();
  await expect(page.getByText("Recent Events", { exact: true })).toBeVisible();
});

// ---- Goals ----

test("goal can be created from the dashboard and is listed", async ({ page }) => {
  await login(page);
  await ensureSite(page);

  await page.goto("/app/goals");
  await expect(page.locator('select[name="site"]')).toBeVisible();
  await expect(page.getByRole("heading", { name: "Create Goal" })).toBeVisible();

  const goalName = `Signup ${Date.now()}`;
  await page.fill('input[name="name"]', goalName);
  await page.fill('input[name="event_name"]', "user_signed_up");
  await page.getByRole("button", { name: "Create Goal" }).click();

  // Lands on the per-site goals page with the new goal listed; scope the
  // event-name check to this goal's row (older runs may leave other goals).
  const row = page.getByRole("row", { name: goalName });
  await expect(row).toBeVisible();
  await expect(row.getByText("user_signed_up")).toBeVisible();
});

// ---- Alerts + channels ----

test("alert channel can be added and an alert created", async ({ page }) => {
  await login(page);
  await ensureSite(page);

  await page.goto("/app/alerts");
  await expect(page.getByRole("heading", { name: "Alert Channels" })).toBeVisible();

  // Add a webhook channel.
  const hookUrl = `https://hooks.e2e.test/${Date.now()}`;
  await page.locator('select[name="kind"]').selectOption("webhook");
  await page.fill('input[name="url"]', hookUrl);
  await page.getByRole("button", { name: "Add Channel" }).click();

  // Channel is listed (in the table cell) with Test + Delete actions.
  await expect(page.getByRole("cell", { name: hookUrl })).toBeVisible();
  await expect(page.getByRole("button", { name: "Test" }).first()).toBeVisible();

  // The create-alert form now has a channel to point at.
  await expect(page.getByRole("heading", { name: "Create Alert" })).toBeVisible();
  await page.locator('select[name="type"]').selectOption("traffic_spike");
  await page.fill('input[name="threshold"]', "200");
  await page.getByRole("button", { name: "Create Alert" }).click();

  // Alert now appears in the alerts table (older runs may leave others).
  await expect(page.getByText("traffic_spike").first()).toBeVisible();
});

test("telegram channel offered in the channel kind selector", async ({
  page,
}) => {
  await login(page);
  await ensureSite(page);

  await page.goto("/app/alerts");
  const kinds = await page
    .locator('select[name="kind"] option')
    .allInnerTexts();
  expect(kinds.join(" ").toLowerCase()).toContain("telegram");
});
