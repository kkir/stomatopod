/**
 * P2 product-confidence e2e: tracker → dashboard data, funnel results UI,
 * API keys mint/revoke/use, site settings, digest, password validation,
 * channel test-fire, overview range/compare/filter, install snippet.
 */
import { test, expect, type Page } from "@playwright/test";
import {
  ADMIN_PASSWORD,
  UI,
  createSite,
  fillNoAutofill,
  login,
  openTrackedFixture,
  postEvent,
  waitForPageviews,
  waitForSpa,
} from "./helpers";

// ---- Tracker → dashboard numbers ----

test("tracker pageview appears on site overview", async ({ page }) => {
  await login(page);
  const uniq = `Track ${Date.now().toString(36)}`;
  const site = await createSite(page, uniq, `track-${Date.now().toString(36)}.example`);

  // Empty overview shows install hero with this site's public key.
  await page.goto(`${UI}/sites/${site.id}`);
  await waitForSpa(page);
  await expect(
    page.getByRole("heading", { name: "Start collecting analytics" }),
  ).toBeVisible({ timeout: 15_000 });
  const snippet = page.locator("pre code");
  await expect(snippet).toContainText(`data-site="${site.public_key}"`);
  await expect(snippet).toContainText("/tracker.js");

  // Load a real page with the live tracker (ingest hits this server).
  await openTrackedFixture(page, site.public_key);
  await waitForPageviews(page.request, site.id, 1);

  // Overview should leave the empty install hero and show a non-zero stat.
  await page.goto(`${UI}/sites/${site.id}?range=7d`);
  await waitForSpa(page);
  await expect(
    page.getByRole("heading", { name: "Start collecting analytics" }),
  ).toHaveCount(0);
  await expect(page.getByText("Pageviews", { exact: true }).first()).toBeVisible();
  // Stat tile value is a font-display number ≥ 1.
  await expect(
    page.locator(".font-display").filter({ hasText: /^[1-9]\d*$/ }).first(),
  ).toBeVisible({ timeout: 15_000 });
});

// ---- Funnel detail conversion UI ----

test("funnel detail shows conversion steps after seeded traffic", async ({
  page,
}) => {
  await login(page);
  const site = await createSite(
    page,
    `FunnelP2 ${Date.now().toString(36)}`,
    `funnel-p2-${Date.now().toString(36)}.example`,
  );

  // Seed sessions: 2 complete pageview→signup, 1 pageview only.
  for (const [i, signup] of [
    [0, true],
    [1, true],
    [2, false],
  ] as const) {
    const ua = `Mozilla/5.0 (X11; Linux x86_64) Chrome/120.0 Session/${i}`;
    await postEvent(
      page.request,
      site.public_key,
      "pageview",
      `https://${site.domain}/start`,
      ua,
    );
    if (signup) {
      await postEvent(
        page.request,
        site.public_key,
        "signup",
        `https://${site.domain}/thanks`,
        ua,
      );
    }
  }
  await waitForPageviews(page.request, site.id, 3);

  await page.goto(`${UI}/sites/${site.id}/funnels`);
  await waitForSpa(page);

  const funnelName = `Signup ${Date.now().toString(36)}`;
  await page.getByPlaceholder("e.g. Signup flow").fill(funnelName);
  const stepLabels = page.getByPlaceholder("Step label");
  await stepLabels.nth(0).fill("Landing");
  await stepLabels.nth(1).fill("Signed up");
  // Second step event name defaults empty — set to signup.
  const eventNames = page.getByPlaceholder("Event name");
  await eventNames.nth(1).fill("signup");
  await page.getByRole("button", { name: "Create funnel" }).click();
  await expect(page.getByText(funnelName)).toBeVisible({ timeout: 10_000 });

  // Open detail via "View →".
  await page.getByText(funnelName).click();
  await waitForSpa(page);
  await expect(page).toHaveURL(new RegExp(`/sites/${site.id}/funnels/`));

  // Bars + accessible data table both expose step labels; assert via the table
  // (role=img summary) so we do not strict-mode-duplicate with visible bars.
  const funnelChart = page.getByRole("img", { name: "Funnel conversion by step" });
  await expect(funnelChart.getByRole("rowheader", { name: "Landing" })).toBeVisible({
    timeout: 15_000,
  });
  await expect(funnelChart.getByRole("rowheader", { name: "Signed up" })).toBeVisible();
  await expect(funnelChart.getByRole("row", { name: /Landing 3/ })).toBeVisible();
  await expect(funnelChart.getByRole("row", { name: /Signed up 2/ })).toBeVisible();
  await expect(page.getByRole("button", { name: "Delete funnel" })).toBeVisible();
  await expect(page.getByRole("link", { name: /All funnels/ })).toBeVisible();
});

// ---- API keys UI mint + revoke + use ----

test("site API key can be minted, used for ingest, and revoked", async ({
  page,
}) => {
  await login(page);
  const site = await createSite(
    page,
    `KeysP2 ${Date.now().toString(36)}`,
    `keys-p2-${Date.now().toString(36)}.example`,
  );

  await page.goto(`${UI}/sites/${site.id}/keys`);
  await waitForSpa(page);
  await expect(page.getByRole("heading", { name: "API Keys" })).toBeVisible();

  const keyName = `ingest-${Date.now().toString(36)}`;
  await page.getByPlaceholder("Key name").fill(keyName);
  // Default scope is ingest.
  await page.getByRole("button", { name: "Create key" }).click();

  // One-time secret banner.
  await expect(page.getByText(/created - copy it now/i)).toBeVisible({
    timeout: 10_000,
  });
  const secretCode = page.locator("div.bg-teal-soft code");
  await expect(secretCode).toBeVisible();
  const secret = (await secretCode.innerText()).trim();
  expect(secret.startsWith("sk_live_")).toBeTruthy();

  // Key appears in the list, not only in the one-time secret banner.
  const keyRow = page
    .locator(".flex.items-center.justify-between.gap-3.py-2.border-t.border-border-1")
    .filter({ hasText: keyName });
  await expect(keyRow).toBeVisible();

  // Use the secret against server-side ingest.
  const ingest = await page.request.post("/api/v1/ingest", {
    headers: {
      authorization: `Bearer ${secret}`,
      "content-type": "application/json",
    },
    data: { name: "p2_key_event", properties: { ok: true } },
  });
  expect(ingest.status()).toBe(204);

  // Revoke.
  await keyRow.getByRole("button", { name: "Revoke" }).click();
  await expect(keyRow).toHaveCount(0, { timeout: 10_000 });

  // Revoked key must fail.
  const after = await page.request.post("/api/v1/ingest", {
    headers: {
      authorization: `Bearer ${secret}`,
      "content-type": "application/json",
    },
    data: { name: "after_revoke" },
  });
  expect(after.status()).toBe(401);
});

// ---- Site settings (name / domain) ----

test("site settings save updates name and domain", async ({ page }) => {
  await login(page);
  const site = await createSite(
    page,
    `SettingsP2 ${Date.now().toString(36)}`,
    `settings-p2-${Date.now().toString(36)}.example`,
  );

  await page.goto(`${UI}/sites/${site.id}/settings`);
  await waitForSpa(page);
  await expect(
    page.getByRole("heading", { name: "Site Settings", level: 1 }),
  ).toBeVisible();

  const newName = `Renamed ${Date.now().toString(36)}`;
  const newDomain = `renamed-${Date.now().toString(36)}.example`;
  // General card: Name + Domain inputs (labelled).
  const nameInput = page.locator('label:has-text("Name") input');
  const domainInput = page.locator('label:has-text("Domain") input');
  await nameInput.fill(newName);
  await domainInput.fill(newDomain);
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByText("Saved", { exact: true })).toBeVisible({
    timeout: 10_000,
  });

  // API reflects the change.
  const res = await page.request.get("/api/v1/sites");
  expect(res.ok()).toBeTruthy();
  const list = await res.json();
  const found = (list.sites as Array<{ id: string; name: string; domain: string }>).find(
    (s) => s.id === site.id,
  );
  expect(found?.name).toBe(newName);
  expect(found?.domain).toBe(newDomain);
});

// ---- Digest subscription UI ----

test("digest subscription can be enabled from site settings", async ({
  page,
}) => {
  await login(page);
  const site = await createSite(
    page,
    `DigestP2 ${Date.now().toString(36)}`,
    `digest-p2-${Date.now().toString(36)}.example`,
  );

  // Digest delivery needs a channel; add a public webhook first.
  await page.goto(`${UI}/sites/${site.id}/settings`);
  await waitForSpa(page);
  await page.getByRole("tab", { name: "Webhook", exact: true }).click();
  const hookUrl = `https://example.com/hooks/digest-${Date.now().toString(36)}`;
  await fillNoAutofill(page, "https://example.com/hooks/stomatopod", hookUrl);
  await page.getByRole("button", { name: "Add webhook" }).click();
  await expect(page.getByText(hookUrl).first()).toBeVisible({
    timeout: 10_000,
  });

  // Enable digest via the switch control.
  await expect(
    page.getByRole("heading", { name: "Analytics digest" }),
  ).toBeVisible();
  await page.getByRole("switch", { name: "Send me a digest" }).click();

  // Frequency select becomes meaningful; Send test appears once subscribed.
  await expect(page.getByRole("button", { name: "Send test" })).toBeVisible({
    timeout: 10_000,
  });

  const sub = await page.request.get(
    `/api/v1/sites/${site.id}/digest-subscription`,
  );
  expect(sub.ok()).toBeTruthy();
  const body = await sub.json();
  expect(body.subscription?.enabled).toBe(true);
});

// ---- Change password UI (client validation only — keep admin creds intact) ----

test("change password form validates length and confirmation", async ({
  page,
}) => {
  await login(page);
  await page.goto(`${UI}/keys`);
  await waitForSpa(page);
  await expect(
    page.getByRole("heading", { name: "Change password" }),
  ).toBeVisible();

  await page.getByPlaceholder("Current password").fill(ADMIN_PASSWORD);
  await page.getByPlaceholder("New password", { exact: true }).fill("short");
  await page.getByPlaceholder("Confirm new password").fill("short");
  await page.getByRole("button", { name: "Update password" }).click();
  await expect(
    page.getByText("New password must be at least 12 characters", { exact: true }),
  ).toBeVisible();

  await page.getByPlaceholder("New password", { exact: true }).fill("long-enough-pass");
  await page.getByPlaceholder("Confirm new password").fill("long-enough-other");
  await page.getByRole("button", { name: "Update password" }).click();
  await expect(
    page.getByText("New password and confirmation do not match"),
  ).toBeVisible();
});

// ---- Channel test-fire from UI ----

test("notification destination test button reports a result", async ({
  page,
}) => {
  await login(page);
  const site = await createSite(
    page,
    `TestFire ${Date.now().toString(36)}`,
    `testfire-${Date.now().toString(36)}.example`,
  );

  await page.goto(`${UI}/sites/${site.id}/settings`);
  await waitForSpa(page);
  await page.getByRole("tab", { name: "Webhook", exact: true }).click();
  const hookUrl = `https://example.com/hooks/tf-${Date.now().toString(36)}`;
  await fillNoAutofill(page, "https://example.com/hooks/stomatopod", hookUrl);
  await page.getByRole("button", { name: "Add webhook" }).click();
  await expect(page.getByText(hookUrl).first()).toBeVisible({
    timeout: 10_000,
  });

  // Test-fire: example.com may return non-2xx → "Test failed"; either way the UI path runs.
  // Accessible name is "Test channel {url}" (aria-label); match by prefix.
  await page.getByRole("button", { name: new RegExp(`^Test channel ${hookUrl}`) }).click();
  await expect(
    page.getByText(/Test (delivered|failed:|error:)/i).first(),
  ).toBeVisible({ timeout: 15_000 });
});

// ---- Overview range / compare / filter wiring ----

test("overview range, compare, and filter update the URL and UI", async ({
  page,
}) => {
  await login(page);
  const site = await createSite(
    page,
    `RangeP2 ${Date.now().toString(36)}`,
    `range-p2-${Date.now().toString(36)}.example`,
  );

  // Seed one pageview so filtered empty state is meaningful.
  await postEvent(
    page.request,
    site.public_key,
    "pageview",
    `https://${site.domain}/home`,
  );
  await waitForPageviews(page.request, site.id, 1);

  await page.goto(`${UI}/sites/${site.id}`);
  await waitForSpa(page);

  // Range pills are a labelled nav of links (accessible names are expanded,
  // e.g. "Last 7 days"), not a tablist.
  await page.getByRole("navigation", { name: "Date range" })
    .getByRole("link", { name: "Last 7 days" })
    .click();
  await expect(page).toHaveURL(/range=7d/);

  // Compare toggle (aria-label describes enable/disable).
  await page.getByRole("link", { name: "Compare to previous period" }).click();
  await expect(page).toHaveURL(/compare=/);
  await expect(
    page.getByRole("link", { name: "Disable period comparison" }),
  ).toBeVisible();

  // Filter form.
  await page.getByRole("button", { name: "Filter", exact: true }).click();
  await page.getByPlaceholder("filter value").fill("/home");
  await page.getByRole("button", { name: "Apply", exact: true }).click();
  await expect(page).toHaveURL(/filter=/);
  // Active filter pill (humanized).
  await expect(page.getByText(/Page|URL|is|\/home/i).first()).toBeVisible({
    timeout: 10_000,
  });
  await expect(page.locator("body")).not.toContainText("Failed to load");
});

// ---- Install snippet documents data-site (global docs + site card covered above) ----

test("docs install section documents data-site attribute", async ({ page }) => {
  await login(page);
  await page.goto(`${UI}/docs`);
  await waitForSpa(page);
  await expect(page.getByRole("heading", { name: "Docs" })).toBeVisible();
  // Markdown docs include the install snippet shape.
  await expect(page.locator("body")).toContainText("data-site");
  await expect(page.locator("body")).toContainText("tracker.js");
});
