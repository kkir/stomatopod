import { test, expect, Page } from "@playwright/test";

// Credentials must match SERVER_ENV in playwright.config.ts.
const ADMIN_EMAIL = "admin@e2e.test";
const ADMIN_PASSWORD = "playwright-test-pw";

// The Dioxus fullstack app owns the dashboard at the site root (SSR +
// hydration). `UI` is the empty prefix so `${UI}/sites/:id` etc. resolve
// against `/`; use `ROOT` for a bare navigation to the dashboard root.
const UI = "";
const ROOT = "/";

/** Log in through the SSR login form; the session cookie then authorizes
 *  both the SPA shell (`require_auth`) and its `/api/v1` calls. */
async function login(page: Page) {
  await page.goto("/login");
  await page.fill('input[name="email"]', ADMIN_EMAIL);
  await page.fill('input[name="password"]', ADMIN_PASSWORD);
  await page.click('button[type="submit"]');
  await expect(page).not.toHaveURL(/\/login/);
}

/** Wait for the WASM bundle to hydrate: the sidebar (kept with the literal
 *  `side-nav` class for exactly this) is present on every page via the
 *  layout Shell. */
async function waitForSpa(page: Page) {
  await expect(page.locator(".side-nav")).toBeVisible({ timeout: 20_000 });
}

/** Create a site via the JSON API using the browser context's session
 *  cookie; returns its id. */
async function createSite(page: Page, name: string, domain: string) {
  const res = await page.request.post("/api/v1/sites", {
    data: { name, domain },
  });
  expect(res.ok()).toBeTruthy();
  const body = await res.json();
  return body.id as string;
}

// ---- Auth gating ----

test("unauthenticated visit to /ui redirects to login", async ({ page }) => {
  await page.goto(ROOT);
  await expect(page).toHaveURL(/\/login/);
});

// ---- SPA shell ----

test("authenticated /ui renders the SPA shell", async ({ page }) => {
  await login(page);
  await page.goto(ROOT);
  await waitForSpa(page);
  await expect(page.getByRole("heading", { name: "Sites" })).toBeVisible();
  // Nav links from the sidebar (kept from the legacy tier2 coverage).
  const nav = page.locator(".side-nav");
  await expect(nav.getByRole("link", { name: "Real-time" })).toBeVisible();
  await expect(nav.getByRole("link", { name: "Goals" })).toBeVisible();
  await expect(nav.getByRole("link", { name: "Alerts" })).toBeVisible();
  await expect(nav.getByRole("link", { name: "Docs" })).toBeVisible();
});

// ---- Sites index + overview ----

test("a created site appears and its overview loads", async ({ page }) => {
  await login(page);
  // Unique name so reruns against the persistent test DB stay deterministic.
  const uniq = `Overview Co ${Date.now().toString(36)}`;
  const siteId = await createSite(page, uniq, "overview.example");

  await page.goto(ROOT);
  await waitForSpa(page);
  await expect(page.getByText(uniq)).toBeVisible();

  // Navigate straight to the overview route (client-side routing).
  await page.goto(`${UI}/sites/${siteId}`);
  await waitForSpa(page);
  await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();
  // The hero stat tiles render even with zero data.
  await expect(page.getByText("Pageviews", { exact: true })).toBeVisible();
  await expect(page.getByText("Bounce Rate", { exact: true })).toBeVisible();
  // Entry/exit panels + export links (ported from legacy tier2 coverage).
  await expect(page.getByRole("heading", { name: "Entry Pages" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Exit Pages" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Export" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Events CSV" })).toBeVisible();
  // Breakdown panels carry the CSV export link (literal `csv-btn` class).
  await expect(page.locator("a.csv-btn").first()).toBeVisible();
  await expect(page.locator("body")).not.toContainText("Failed to load");
});

// ---- CSV export (server endpoint, unchanged by the SPA cutover) ----

test("top-pages CSV export downloads as text/csv", async ({ page }) => {
  await login(page);
  const siteId = await createSite(page, "CSV Co", "csv.example");

  const res = await page.request.get(
    `/api/v1/sites/${siteId}/top-pages?range=30d&format=csv`,
  );
  expect(res.status()).toBe(200);
  expect(res.headers()["content-type"] ?? "").toContain("csv");
  expect(res.headers()["content-disposition"] ?? "").toContain("top-pages.csv");
  expect(await res.text()).toContain("value,pageviews,sessions,pct");
});

// ---- Per-site tabs ----

test("all per-site tabs render without error", async ({ page }) => {
  await login(page);
  const siteId = await createSite(page, "Tabs Co", "tabs.example");

  const tabs: Array<[string, RegExp]> = [
    [`${UI}/sites/${siteId}`, /Overview/],
    [`${UI}/sites/${siteId}/realtime`, /Real-time/],
    [`${UI}/sites/${siteId}/events`, /Events/],
    [`${UI}/sites/${siteId}/goals`, /Goals/],
    [`${UI}/sites/${siteId}/funnels`, /Funnels/],
    [`${UI}/sites/${siteId}/alerts`, /Alerts/],
    [`${UI}/sites/${siteId}/keys`, /API Keys/],
    [`${UI}/sites/${siteId}/settings`, /Site Settings/],
  ];

  for (const [url, heading] of tabs) {
    await page.goto(url);
    await waitForSpa(page);
    await expect(page.getByRole("heading", { name: heading }).first()).toBeVisible();
    await expect(page.locator("body")).not.toContainText("Failed to load");
  }
});

// ---- Global insight + management pages ----

test("global pages render without error", async ({ page }) => {
  await login(page);
  // Ensure at least one site exists so the site-scoped globals have data.
  await createSite(page, "Global Co", "global.example");

  const pages: Array<[string, RegExp]> = [
    [`${UI}/realtime`, /Real-time/],
    [`${UI}/goals`, /Goals/],
    [`${UI}/campaigns`, /Campaigns/],
    [`${UI}/retention`, /Retention/],
    [`${UI}/paths`, /Paths/],
    [`${UI}/compare`, /Compare/],
    [`${UI}/alerts`, /Alerts/],
    [`${UI}/keys`, /API Keys/],
  ];

  for (const [url, heading] of pages) {
    await page.goto(url);
    await waitForSpa(page);
    await expect(page.getByRole("heading", { name: heading }).first()).toBeVisible();
    await expect(page.locator("body")).not.toContainText("Failed to load");
  }
});

// ---- Docs ----

test("docs page injects rendered markdown", async ({ page }) => {
  await login(page);
  await page.goto(`${UI}/docs`);
  await waitForSpa(page);
  await expect(page.getByRole("heading", { name: "Docs" })).toBeVisible();
  // The server-rendered markdown includes the documentation title.
  await expect(page.locator("body")).toContainText(/Stomatopod/i);
});

// ---- Funnel builder (SPA interactivity) ----

test("funnel builder creates a funnel", async ({ page }) => {
  await login(page);
  const siteId = await createSite(page, "Funnel Co", "funnel.example");

  await page.goto(`${UI}/sites/${siteId}/funnels`);
  await waitForSpa(page);

  const funnelName = `Signup path ${Date.now().toString(36)}`;
  await page.getByPlaceholder("e.g. Signup flow").fill(funnelName);
  // Two default steps: fill their labels.
  const stepLabels = page.getByPlaceholder("Step label");
  await stepLabels.nth(0).fill("Landing");
  await stepLabels.nth(1).fill("Converted");

  await page.getByRole("button", { name: "Create funnel" }).click();

  // The new funnel appears in the list above the builder.
  await expect(page.getByText(funnelName)).toBeVisible({ timeout: 10_000 });
});

// ---- Goals (SPA interactivity, ported from legacy tier2) ----

test("a goal can be created and is listed", async ({ page }) => {
  await login(page);
  const siteId = await createSite(page, "Goals Co", "goals.example");

  await page.goto(`${UI}/sites/${siteId}/goals`);
  await waitForSpa(page);

  const goalName = `Signup ${Date.now().toString(36)}`;
  await page.getByPlaceholder("Goal name").fill(goalName);
  await page.getByPlaceholder(/Event name/).fill("user_signed_up");
  await page.getByRole("button", { name: "Add goal" }).click();

  await expect(page.getByText(goalName)).toBeVisible({ timeout: 10_000 });
  await expect(page.getByText("Event: user_signed_up")).toBeVisible();
});

// ---- Alerts + channels (SPA interactivity, ported from legacy tier2) ----

test("an alert channel can be added and an alert created", async ({ page }) => {
  await login(page);
  const siteId = await createSite(page, "Alerts Co", "alerts.example");

  await page.goto(`${UI}/sites/${siteId}/alerts`);
  await waitForSpa(page);
  await expect(page.getByRole("heading", { name: "Channels" })).toBeVisible();

  // The channel kind selector offers telegram (ported tier2 assertion).
  const kinds = await page
    .locator("select")
    .first()
    .locator("option")
    .allInnerTexts();
  expect(kinds.join(" ").toLowerCase()).toContain("telegram");

  // Add a webhook channel.
  const hookUrl = `https://hooks.e2e.test/${Date.now().toString(36)}`;
  await page.getByPlaceholder("Webhook URL / chat id").fill(hookUrl);
  await page.getByRole("button", { name: "Add channel" }).click();
  // The URL shows both in the channel list and later in the alert form's
  // channel <option>; the list entry is the first match.
  await expect(page.getByText(hookUrl).first()).toBeVisible({ timeout: 10_000 });

  // With a channel present, create a traffic-spike alert. Assert on the
  // created row's config line ("threshold 200 · 60m"), which is unique to the
  // alerts list (the kind label also appears as a hidden <option>).
  await page.getByPlaceholder("threshold").fill("200");
  await page.getByRole("button", { name: "Add alert" }).click();
  await expect(page.getByText(/threshold 200/)).toBeVisible({
    timeout: 10_000,
  });
});
