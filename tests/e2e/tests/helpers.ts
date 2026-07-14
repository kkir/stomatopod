import { expect, type APIRequestContext, type Page } from "@playwright/test";

export const ADMIN_EMAIL = "admin@e2e.test";
export const ADMIN_PASSWORD = "playwright-test-pw";

export const UI = "";
export const ROOT = "/";

const CHROME_UA =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/** Log in through the SSR login form. */
export async function login(page: Page) {
  await page.goto("/login");
  await page.fill('input[name="email"]', ADMIN_EMAIL);
  await page.fill('input[name="password"]', ADMIN_PASSWORD);
  await page.click('button[type="submit"]');
  await expect(page).not.toHaveURL(/\/login/);
}

/** Wait for WASM hydration before interacting with the SPA. */
export async function waitForSpa(page: Page) {
  await expect(page.locator(".side-nav")).toBeVisible({ timeout: 20_000 });
  await expect(page.locator("[data-hydrated='true']")).toBeAttached({
    timeout: 20_000,
  });
}

export type CreatedSite = {
  id: string;
  domain: string;
  name: string;
  public_key: string;
};

/** Create a site via the JSON API using the browser session cookie. */
export async function createSite(
  page: Page,
  name: string,
  domain: string,
): Promise<CreatedSite> {
  const res = await page.request.post("/api/v1/sites", {
    data: { name, domain },
  });
  expect(res.ok()).toBeTruthy();
  const body = await res.json();
  return {
    id: body.id as string,
    domain: body.domain as string,
    name: body.name as string,
    public_key: body.public_key as string,
  };
}

/**
 * Fill a Settings "NoAutofillInput" field (readonly until focused).
 */
export async function fillNoAutofill(
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

/** POST a browser-like pageview (or custom event) into the live e2e server. */
export async function postEvent(
  request: APIRequestContext,
  siteKey: string,
  name: string,
  url: string,
  ua = CHROME_UA,
) {
  const res = await request.post("/api/v1/event", {
    headers: {
      "content-type": "application/json",
      "user-agent": ua,
    },
    data: { k: siteKey, n: name, u: url, w: 1920, h: 1080 },
  });
  expect(res.status()).toBe(204);
}

/** Poll pageviews until total_pageviews >= want (or timeout). */
export async function waitForPageviews(
  request: APIRequestContext,
  siteId: string,
  want: number,
  timeoutMs = 15_000,
) {
  await expect
    .poll(
      async () => {
        const res = await request.get(
          `/api/v1/sites/${siteId}/pageviews?range=7d`,
        );
        if (!res.ok()) return 0;
        const json = await res.json();
        return (json.total_pageviews as number) ?? 0;
      },
      {
        message: `expected >= ${want} pageviews`,
        timeout: timeoutMs,
        intervals: [100, 200, 400, 800],
      },
    )
    .toBeGreaterThanOrEqual(want);
}

/**
 * Serve a minimal page that loads the real /tracker.js and posts to the real
 * ingest endpoint (no request interception on /api/v1/event).
 */
export async function openTrackedFixture(
  page: Page,
  siteKey: string,
  path = "/tracked-fixture-live",
) {
  await page.route(`**${path}`, async (route) => {
    const origin = new URL(route.request().url()).origin;
    const html = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <script defer src="${origin}/tracker.js"
    data-api="${origin}/api/v1/event"
    data-site="${siteKey}"></script>
</head>
<body>
  <h1>Live tracked fixture</h1>
  <div id="ready">ok</div>
</body>
</html>`;
    await route.fulfill({
      status: 200,
      contentType: "text/html; charset=utf-8",
      body: html,
    });
  });
  await page.goto(path);
  await expect(page.locator("#ready")).toBeVisible();
}
