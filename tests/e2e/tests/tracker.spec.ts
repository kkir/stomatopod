import { test, expect, type Page, type Request } from "@playwright/test";

/** Captured ingest payloads from the tracker. */
type EventPayload = {
  k: string;
  n: string;
  u: string;
  p?: unknown;
  t?: number;
};

function isIngestRequest(req: Request, path = "/api/v1/event"): boolean {
  if (req.method() !== "POST") return false;
  try {
    return new URL(req.url()).pathname === path;
  } catch {
    return false;
  }
}

/** Intercept ingest POSTs (including sendBeacon) and record JSON bodies. */
async function captureIngest(
  page: Page,
  path = "/api/v1/event",
): Promise<EventPayload[]> {
  const events: EventPayload[] = [];
  await page.route(`**${path}`, async (route) => {
    if (!isIngestRequest(route.request(), path)) {
      await route.continue();
      return;
    }
    const raw = route.request().postData();
    if (raw) {
      try {
        events.push(JSON.parse(raw) as EventPayload);
      } catch {
        /* ignore non-JSON */
      }
    }
    await route.fulfill({ status: 204, body: "" });
  });
  return events;
}

type FixtureOpts = {
  /** When false, omit data-api so the tracker must derive it from script.src. */
  includeDataApi?: boolean;
  /** Extra attributes on the script tag (e.g. data-exclude). */
  scriptAttrs?: string;
  /** HTML injected before the tracker script (e.g. a call queue). */
  headPrefix?: string;
  /** Second tracker script tag (double-include regression). */
  doubleInclude?: boolean;
  siteKey?: string;
  path?: string;
};

/**
 * Serve a minimal page that loads /tracker.js from the e2e server origin.
 * Returns after the document body is ready (tracker uses defer).
 */
async function openTrackedFixture(page: Page, opts: FixtureOpts = {}) {
  const path = opts.path ?? "/tracked-fixture";
  const siteKey = opts.siteKey ?? "e2e-tracker-site-key";
  const includeDataApi = opts.includeDataApi !== false;
  const extra = opts.scriptAttrs ? ` ${opts.scriptAttrs}` : "";

  await page.route(`**${path}`, async (route) => {
    const origin = new URL(route.request().url()).origin;
    const dataApi = includeDataApi
      ? ` data-api="${origin}/api/v1/event"`
      : "";
    const scriptTag = `<script defer src="${origin}/tracker.js"${dataApi} data-site="${siteKey}"${extra}></script>`;
    const html = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  ${opts.headPrefix ?? ""}
  ${scriptTag}
  ${opts.doubleInclude ? scriptTag : ""}
</head>
<body>
  <h1>Tracked fixture</h1>
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

function pageviews(events: EventPayload[]) {
  return events.filter((e) => e.n === "pageview");
}

async function waitForPageviewCount(events: EventPayload[], n: number) {
  await expect
    .poll(() => pageviews(events).length, {
      message: `expected ${n} pageview(s), got payloads: ${JSON.stringify(events)}`,
      timeout: 5_000,
    })
    .toBe(n);
}

// ---- Initial load ----

test("fires exactly one pageview on load", async ({ page }) => {
  const events = await captureIngest(page);
  await openTrackedFixture(page);
  await waitForPageviewCount(events, 1);

  const pv = pageviews(events)[0];
  expect(pv.k).toBe("e2e-tracker-site-key");
  expect(pv.u).toContain("/tracked-fixture");
  expect(typeof pv.t).toBe("number");
});

test("fires initial pageview when data-api is omitted (derived from script src)", async ({
  page,
}) => {
  const events = await captureIngest(page);
  await openTrackedFixture(page, { includeDataApi: false });
  await waitForPageviewCount(events, 1);
  expect(pageviews(events)[0].u).toContain("/tracked-fixture");
});

test("data-api overrides the ingest URL", async ({ page }) => {
  const customPath = "/custom-ingest-endpoint";
  const events = await captureIngest(page, customPath);

  await page.route("**/tracked-fixture-custom-api", async (route) => {
    const origin = new URL(route.request().url()).origin;
    const html = `<!DOCTYPE html>
<html><head>
  <script defer src="${origin}/tracker.js"
    data-api="${origin}${customPath}"
    data-site="custom-api-key"></script>
</head><body><div id="ready">ok</div></body></html>`;
    await route.fulfill({
      status: 200,
      contentType: "text/html",
      body: html,
    });
  });

  // Default path must receive nothing.
  const defaultEvents = await captureIngest(page, "/api/v1/event");

  await page.goto("/tracked-fixture-custom-api");
  await expect(page.locator("#ready")).toBeVisible();
  await waitForPageviewCount(events, 1);
  expect(pageviews(events)[0].k).toBe("custom-api-key");
  expect(defaultEvents.length).toBe(0);
});

// ---- Scroll / replaceState spam (the original bug) ----

test("hash-only replaceState (scroll-spy) does not fire extra pageviews", async ({
  page,
}) => {
  const events = await captureIngest(page);
  await openTrackedFixture(page);
  await waitForPageviewCount(events, 1);

  // Simulate scroll-spy / section highlighting: many replaceState hash updates.
  await page.evaluate(() => {
    for (let i = 0; i < 25; i++) {
      history.replaceState(null, "", `/tracked-fixture#section-${i}`);
    }
  });
  // Allow the tracker's setTimeout(0) hooks to run.
  await page.evaluate(() => new Promise((r) => setTimeout(r, 50)));

  expect(pageviews(events)).toHaveLength(1);
});

test("replaceState that only repeats the same path does not fire again", async ({
  page,
}) => {
  const events = await captureIngest(page);
  await openTrackedFixture(page);
  await waitForPageviewCount(events, 1);

  await page.evaluate(() => {
    history.replaceState({ x: 1 }, "", "/tracked-fixture");
    history.replaceState({ x: 2 }, "", "/tracked-fixture");
    history.replaceState({ x: 3 }, "", "/tracked-fixture?keep=1");
    history.replaceState({ x: 4 }, "", "/tracked-fixture?keep=1");
  });
  await page.evaluate(() => new Promise((r) => setTimeout(r, 50)));

  // First replaceStates share path with initial load; only ?keep=1 is new.
  await waitForPageviewCount(events, 2);
  expect(pageviews(events)[1].u).toContain("keep=1");
});

// ---- Real SPA navigations ----

test("pushState to a new path fires a pageview", async ({ page }) => {
  const events = await captureIngest(page);
  await openTrackedFixture(page);
  await waitForPageviewCount(events, 1);

  await page.evaluate(() => {
    history.pushState(null, "", "/spa-route-a");
  });
  await waitForPageviewCount(events, 2);
  expect(pageviews(events)[1].u).toContain("/spa-route-a");
});

test("replaceState to a new path fires a pageview", async ({ page }) => {
  const events = await captureIngest(page);
  await openTrackedFixture(page);
  await waitForPageviewCount(events, 1);

  await page.evaluate(() => {
    history.replaceState(null, "", "/spa-replaced");
  });
  await waitForPageviewCount(events, 2);
  expect(pageviews(events)[1].u).toContain("/spa-replaced");
});

test("popstate (back) to a prior path fires a pageview", async ({ page }) => {
  const events = await captureIngest(page);
  await openTrackedFixture(page);
  await waitForPageviewCount(events, 1);

  await page.evaluate(() => {
    history.pushState(null, "", "/spa-forward");
  });
  await waitForPageviewCount(events, 2);

  await page.evaluate(() => {
    history.back();
  });
  // Back to /tracked-fixture - a path change relative to lastPath (/spa-forward).
  await waitForPageviewCount(events, 3);
  expect(pageviews(events)[2].u).toContain("/tracked-fixture");
});

test("navigating to the same path twice does not double-count", async ({
  page,
}) => {
  const events = await captureIngest(page);
  await openTrackedFixture(page);
  await waitForPageviewCount(events, 1);

  await page.evaluate(() => {
    history.pushState(null, "", "/same-twice");
    history.pushState(null, "", "/same-twice");
  });
  await page.evaluate(() => new Promise((r) => setTimeout(r, 50)));
  await waitForPageviewCount(events, 2);
});

// ---- Custom events & queue ----

test("stomatopod('event', ...) sends a custom event", async ({ page }) => {
  const events = await captureIngest(page);
  await openTrackedFixture(page);
  await waitForPageviewCount(events, 1);

  await page.evaluate(() => {
    (window as unknown as { stomatopod: (...args: unknown[]) => void }).stomatopod(
      "event",
      "signup",
      { plan: "pro" },
    );
  });

  await expect
    .poll(() => events.filter((e) => e.n === "signup").length)
    .toBe(1);
  const signup = events.find((e) => e.n === "signup")!;
  expect(signup.p).toEqual({ plan: "pro" });
  // Custom events must not advance the pageview dedupe key incorrectly:
  // same path push should still not add a pageview.
  expect(pageviews(events)).toHaveLength(1);
});

test("drains stomatopod.push queue from before the script loaded", async ({
  page,
}) => {
  const events = await captureIngest(page);
  await openTrackedFixture(page, {
    headPrefix: `<script>
      window.stomatopod = window.stomatopod || [];
      window.stomatopod.push(["event", "queued_early", { src: "head" }]);
    </script>`,
  });

  await waitForPageviewCount(events, 1);
  await expect
    .poll(() => events.filter((e) => e.n === "queued_early").length)
    .toBe(1);
});

// ---- Guards ----

test("loading the tracker twice only sends one initial pageview", async ({
  page,
}) => {
  const events = await captureIngest(page);
  await openTrackedFixture(page, { doubleInclude: true });
  await waitForPageviewCount(events, 1);
  await page.evaluate(() => new Promise((r) => setTimeout(r, 100)));
  expect(pageviews(events)).toHaveLength(1);
});

test("data-exclude disables tracking entirely", async ({ page }) => {
  const events = await captureIngest(page);
  await openTrackedFixture(page, { scriptAttrs: "data-exclude" });
  await page.evaluate(() => new Promise((r) => setTimeout(r, 150)));
  expect(events).toHaveLength(0);
});

test("install snippet shape documents data-api (served tracker is usable with it)", async ({
  request,
}) => {
  // Contract: docs (llms.txt) show data-api in the install snippet so operators
  // do not ship a page-relative /api/v1/event by accident.
  const docs = await request.get("/llms.txt");
  expect(docs.status()).toBe(200);
  const body = await docs.text();
  expect(body).toContain("data-api=");
  expect(body).toContain("data-site=");
  expect(body).toMatch(/data-api="https:\/\/your-host\/api\/v1\/event"/);
});
