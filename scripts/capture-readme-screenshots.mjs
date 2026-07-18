/**
 * Capture README storefront screenshots from a running instance.
 *
 * Prerequisites: fresh UI + server, e.g.
 *   mise run ui:build
 *   rm -rf data/e2e && mkdir -p data/e2e
 *   DIOXUS_PUBLIC_PATH=$PWD/target/dx/stomatopod/debug/web/public \
 *     STOMATOPOD_ADMIN_EMAIL=admin@e2e.test \
 *     STOMATOPOD_ADMIN_PASSWORD=playwright-test-pw \
 *     ./target/dx/stomatopod/debug/web/server --config tests/e2e/stomatopod.toml serve
 *
 * Then:
 *   BASE_URL=http://127.0.0.1:18080 node scripts/capture-readme-screenshots.mjs
 */
import { chromium } from "../tests/e2e/node_modules/playwright/index.mjs";
import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, "..");
const outDir = path.join(root, "docs/images");
const base = (process.env.BASE_URL ?? "http://127.0.0.1:18080").replace(
  /\/$/,
  "",
);
const email = process.env.STOMATOPOD_ADMIN_EMAIL ?? "admin@e2e.test";
const password = process.env.STOMATOPOD_ADMIN_PASSWORD ?? "playwright-test-pw";
// Prefer auto-detect of /app vs / after login.
let app = process.env.STOMATOPOD_APP_PREFIX ?? "";

fs.mkdirSync(outDir, { recursive: true });

const CHROME_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

async function postEvent(request, siteKey, name, url, extra = {}) {
  // Browser ingest body: k/n/u + optional r/w/h/l/p/t (no client session id).
  const res = await request.post(`${base}/api/v1/event`, {
    headers: {
      "content-type": "application/json",
      "user-agent": extra.ua ?? CHROME_UA,
    },
    data: {
      k: siteKey,
      n: name,
      u: url,
      r: extra.referrer,
      w: 1440,
      h: 900,
    },
  });
  if (!res.ok()) {
    throw new Error(`event ${res.status()}: ${await res.text()}`);
  }
}

async function waitForShell(page) {
  await page.waitForSelector(".side-nav", { timeout: 45_000 });
  await page.waitForTimeout(2500);
}

async function main() {
  const browser = await chromium.launch();
  const context = await browser.newContext({
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 2,
  });
  const page = await context.newPage();
  page.on("pageerror", (e) => console.log("pageerror", e.message));

  await page.goto(`${base}/login`);
  await page.fill('input[name="email"]', email);
  await page.fill('input[name="password"]', password);
  await page.click('button[type="submit"]');
  await page.waitForURL((u) => !u.pathname.includes("/login"), {
    timeout: 20_000,
  });
  console.log("logged in at", page.url());

  // Detect dashboard prefix (/app vs root).
  if (!app) {
    const pathName = new URL(page.url()).pathname;
    if (pathName.startsWith("/app")) app = "/app";
    else app = "";
  }
  console.log("app prefix", JSON.stringify(app));

  await waitForShell(page);

  // Create a clean demo site
  const res = await page.request.post(`${base}/api/v1/sites`, {
    data: { name: "Acme Docs", domain: "docs.acme.example" },
  });
  if (!res.ok()) {
    throw new Error(`create site: ${res.status()} ${await res.text()}`);
  }
  const site = await res.json();
  const siteId = site.id;
  const publicKey = site.public_key;
  console.log("site", siteId, publicKey);
  if (!publicKey) throw new Error("site missing public_key");

  const domain = "docs.acme.example";
  const paths = [
    "/",
    "/pricing",
    "/docs",
    "/docs/getting-started",
    "/blog",
    "/signup",
    "/app",
    "/app/dashboard",
  ];
  const referrers = [
    undefined,
    "https://news.ycombinator.com/",
    "https://www.reddit.com/r/selfhosted/",
    "https://github.com/",
    "https://t.co/abc",
  ];

  // Seed pageviews with varied paths/referrers/UAs (UA variance ≈ sessions)
  for (let i = 0; i < 200; i++) {
    const p = paths[i % paths.length];
    const ua = CHROME_UA.replace("120.0.0.0", `${100 + (i % 40)}.0.0.0`);
    await postEvent(
      page.request,
      publicKey,
      "pageview",
      `https://${domain}${p}`,
      {
        referrer: referrers[i % referrers.length],
        ua,
      },
    );
  }

  // Funnel steps (browser ingest; sessionisation is server-side)
  for (let i = 0; i < 60; i++) {
    const ua = CHROME_UA.replace("Chrome/120", `Chrome/${110 + (i % 20)}`);
    await postEvent(page.request, publicKey, "pageview", `https://${domain}/`, {
      ua,
    });
    if (i < 35) {
      await postEvent(
        page.request,
        publicKey,
        "signup_start",
        `https://${domain}/signup`,
        { ua },
      );
    }
    if (i < 18) {
      await postEvent(
        page.request,
        publicKey,
        "signup_complete",
        `https://${domain}/app`,
        { ua },
      );
    }
  }
  console.log("seeded events");

  const funnelRes = await page.request.post(
    `${base}/api/v1/sites/${siteId}/funnels`,
    {
      data: {
        name: "Signup",
        steps: [
          { name: "Landing", event_name: "pageview", filters: [] },
          { name: "Signup", event_name: "signup_start", filters: [] },
          { name: "Activated", event_name: "signup_complete", filters: [] },
        ],
      },
    },
  );
  if (!funnelRes.ok()) {
    console.log("funnel create", funnelRes.status(), await funnelRes.text());
  } else {
    const funnel = await funnelRes.json();
    console.log("funnel", funnel.id ?? funnel);
    var funnelId = funnel.id;
  }

  // Allow ingest flush (e2e config is aggressive)
  await page.waitForTimeout(4000);

  // 1) Overview
  await page.goto(`${base}${app}/sites/${siteId}`);
  await waitForShell(page);
  // Wait for non-zero pageviews if possible
  try {
    await page.waitForFunction(
      () => {
        const t = document.body?.innerText || "";
        return /PAGEVIEWS[\s\S]{0,40}[1-9]\d*/i.test(t);
      },
      { timeout: 15_000 },
    );
  } catch {
    console.log("warning: pageviews may still be zero");
  }
  await page.waitForTimeout(1500);
  await page.screenshot({
    path: path.join(outDir, "dashboard-overview.png"),
    fullPage: false,
  });
  console.log("wrote dashboard-overview.png");

  // 2) Funnel detail if we have an id, else list
  if (funnelId) {
    await page.goto(`${base}${app}/sites/${siteId}/funnels/${funnelId}`);
  } else {
    await page.goto(`${base}${app}/sites/${siteId}/funnels`);
  }
  await waitForShell(page);
  await page.waitForTimeout(2000);
  await page.screenshot({
    path: path.join(outDir, "funnels.png"),
    fullPage: false,
  });
  console.log("wrote funnels.png");

  // 3) Empty site → install / start collecting card (if rendered)
  const emptyRes = await page.request.post(`${base}/api/v1/sites`, {
    data: { name: "New Property", domain: "new.example" },
  });
  if (emptyRes.ok()) {
    const empty = await emptyRes.json();
    await page.goto(`${base}${app}/sites/${empty.id}`);
    await waitForShell(page);
    const install = page
      .locator("text=Start collecting")
      .or(page.locator("text=Install the tracker"))
      .or(page.locator('pre[aria-label="Tracker install snippet"]'));
    if (await install.count()) {
      await install.first().scrollIntoViewIfNeeded();
      await page.waitForTimeout(500);
      await page.screenshot({
        path: path.join(outDir, "tracker-install.png"),
        fullPage: false,
      });
      console.log("wrote tracker-install.png");
    } else {
      console.log("install card not visible; skipping tracker-install.png");
    }
  }

  await browser.close();
  console.log("done →", outDir);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
