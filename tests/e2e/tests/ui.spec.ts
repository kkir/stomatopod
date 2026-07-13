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

/** Wait for the WASM bundle to hydrate. The sidebar (kept with the literal
 *  `side-nav` class for exactly this) is SSR'd and present before hydration,
 *  so it only proves the shell rendered. The Shell's mount effect sets
 *  `data-hydrated` once wasm has hydrated and event handlers are attached;
 *  wait for that before interacting, or clicks race the hydration. */
async function waitForSpa(page: Page) {
  await expect(page.locator(".side-nav")).toBeVisible({ timeout: 20_000 });
  await expect(page.locator("[data-hydrated='true']")).toBeAttached({
    timeout: 20_000,
  });
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

/**
 * Fill a Settings "NoAutofillInput" field. Those inputs stay `readonly`
 * until focused (anti-password-manager), so a bare Playwright `fill` fails.
 */
async function fillNoAutofill(
  page: Page,
  placeholder: string,
  value: string,
) {
  const input = page.getByPlaceholder(placeholder);
  await expect(input).toBeVisible();
  await input.click();
  await expect(input).not.toHaveAttribute("readonly");
  await input.fill(value);
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
  // Site tab "Overview" is the active insight tab; hero stats render even with zero data.
  await expect(page.getByRole("link", { name: "Overview" })).toBeVisible();
  await expect(page.getByText("Pageviews", { exact: true })).toBeVisible();
  await expect(page.getByText("Bounce Rate", { exact: true })).toBeVisible();
  // Progressive layout: tabbed breakdown cards instead of a wall of tables.
  // `exact: true` so "Pages" does not also match empty-state "No pages yet".
  await expect(
    page.getByRole("heading", { name: "Pages", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Sources", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Locations", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Technology", exact: true }),
  ).toBeVisible();
  // Entry/exit live under Pages dimension tabs (not always mounted).
  await page.getByRole("button", { name: "Entry", exact: true }).click();
  await expect(page.getByText("No Entry pages yet")).toBeVisible();
  await page.getByRole("button", { name: "Exit", exact: true }).click();
  await expect(page.getByText("No Exit pages yet")).toBeVisible();
  // Export is behind a disclosure control, not a permanent card.
  await page.getByRole("button", { name: "Export", exact: true }).click();
  await expect(page.getByRole("link", { name: "Events CSV" })).toBeVisible();
  // Active dimension cards expose a compact CSV link.
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
    [`${UI}/sites/${siteId}`, /Tabs Co/],
    [`${UI}/sites/${siteId}/events`, /Events/],
    [`${UI}/sites/${siteId}/funnels`, /Funnels/],
    [`${UI}/sites/${siteId}/campaigns`, /Tabs Co/],
    [`${UI}/sites/${siteId}/alerts`, /Alerts/],
    [`${UI}/sites/${siteId}/keys`, /API Keys/],
    [`${UI}/sites/${siteId}/settings`, /Site Settings/],
  ];

  for (const [url, heading] of tabs) {
    await page.goto(url);
    await waitForSpa(page);
    await expect(
      page.getByRole("heading", { name: heading }).first(),
    ).toBeVisible();
    await expect(page.locator("body")).not.toContainText("Failed to load");
  }
});

// ---- Global management pages ----

test("global pages render without error", async ({ page }) => {
  await login(page);

  const pages: Array<[string, RegExp]> = [
    [`${UI}/keys`, /API Keys/],
    [`${UI}/docs`, /Docs/],
  ];

  for (const [url, heading] of pages) {
    await page.goto(url);
    await waitForSpa(page);
    await expect(
      page.getByRole("heading", { name: heading }).first(),
    ).toBeVisible();
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

// ---- Notifications (settings) + alerts ----
// Channels live under Site Settings; Alerts only pick among existing
// destinations.

test("a notification destination can be added and an alert created", async ({
  page,
}) => {
  await login(page);
  const siteId = await createSite(page, "Alerts Co", "alerts.example");

  // Notification destinations: Settings → Notifications (Telegram / Slack / Webhook).
  await page.goto(`${UI}/sites/${siteId}/settings`);
  await waitForSpa(page);
  await expect(
    page.getByRole("heading", { name: "Site Settings", level: 1 }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Notifications" }),
  ).toBeVisible();

  // Switch to the Webhook tab (Telegram is the default).
  await page.getByRole("button", { name: "Webhook", exact: true }).click();

  // Host must resolve to a public address so the server's SSRF checks accept
  // it (fake TLDs like .e2e.test fail DNS and are rejected).
  const hookUrl = `https://example.com/hooks/${Date.now().toString(36)}`;
  await fillNoAutofill(
    page,
    "https://example.com/hooks/stomatopod",
    hookUrl,
  );
  await page.getByRole("button", { name: "Add webhook" }).click();
  await expect(page.getByText(hookUrl).first()).toBeVisible({
    timeout: 10_000,
  });

  // With a channel present, create a traffic-spike alert on the Alerts tab.
  // PageHead is h1 "Alerts" and the card is also h2 "Alerts" — pick level 1.
  // New sites already have starter alerts; Add alert is enabled once a
  // notification destination exists.
  await page.goto(`${UI}/sites/${siteId}/alerts`);
  await waitForSpa(page);
  await expect(
    page.getByRole("heading", { name: "Alerts", level: 1 }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Add alert" })).toBeEnabled({
    timeout: 10_000,
  });
  await page.getByLabel(/Threshold/).fill("200");
  await page.getByRole("button", { name: "Add alert" }).click();
  await expect(page.getByText(/200% pageviews above/)).toBeVisible({
    timeout: 10_000,
  });
});
