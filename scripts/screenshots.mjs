#!/usr/bin/env node
// Take the screenshots that README.md and TOUR.md show.
//
// Usage: bun run screenshots           (or: node scripts/screenshots.mjs)
//
// What it does:
// 1. Writes the dummy board to docs/demo/ (scripts/demo-fixtures.mjs).
// 2. Starts the Vite dev server with KARI_FIXTURES=docs/demo on port 1421.
// 3. Opens the board in headless Chromium in a 1920x1080 window. The dummy board
//    holds three nodes on two Claude Code accounts, so the images show the hub:
//    node chips, node names on the cards, and a stats row per account with the
//    two machines that share one of them.
// 4. Pins the clock to the fixture time, so relative times read the same in every release.
// 5. Saves one PNG per view to docs/screenshots/ and writes the app version to docs/screenshots/VERSION.
//
// scripts/bump-version.sh runs this. The release workflow refuses a tag whose
// version does not match docs/screenshots/VERSION.
//
// Needs the Playwright Chromium build once: bunx playwright install chromium

import { spawn } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { chromium } from "playwright";
import { NOW, writeFixtures } from "./demo-fixtures.mjs";

const PORT = 1421;
const URL = `http://localhost:${PORT}/`;
const OUT = "docs/screenshots";
const version = JSON.parse(readFileSync("package.json", "utf8")).version;

mkdirSync(OUT, { recursive: true });

const vite = spawn("bunx", ["vite", "--port", String(PORT), "--strictPort"], {
  env: { ...process.env, KARI_FIXTURES: "docs/demo" },
  stdio: ["ignore", "pipe", "inherit"],
});
vite.stdout.on("data", () => {});
const stop = () => {
  if (!vite.killed) vite.kill();
};
process.on("exit", stop);

async function waitForServer() {
  for (let i = 0; i < 100; i++) {
    try {
      const r = await fetch(URL);
      if (r.ok) return;
    } catch {
      // not up yet
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`vite did not answer on ${URL}`);
}

/** Open the board in a fresh page and wait until it rendered with fonts.
 *  1920x1080 holds all six default columns. The header images are 2x for retina, the tour images 1x. */
async function openBoard(browser, colorScheme, scale = 1) {
  const context = await browser.newContext({
    viewport: { width: 1920, height: 1080 },
    deviceScaleFactor: scale,
    colorScheme,
    timezoneId: "UTC",
    locale: "en-US",
    reducedMotion: "reduce",
  });
  const page = await context.newPage();
  await page.clock.setFixedTime(new Date(NOW));
  await page.goto(URL);
  await page.locator(".card").first().waitFor();
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(300);
  return { context, page };
}

const shot = (page, name) => page.screenshot({ path: join(OUT, `${name}.png`), animations: "disabled" });

async function main() {
  writeFixtures("docs/demo");
  await waitForServer();
  const browser = await chromium.launch();
  try {
    // README header: the board without the plan panel, light and dark, at 2x.
    for (const scheme of ["light", "dark"]) {
      const { context, page } = await openBoard(browser, scheme, 2);
      await page.locator(".proposal").getByRole("button", { name: "Close" }).click();
      await shot(page, scheme === "dark" ? "board-dark" : "board");
      await context.close();
    }

    const { context, page } = await openBoard(browser, "light");

    // The plan panel is open on load because the fixture carries a proposal.
    await shot(page, "plan");
    await page.locator(".proposal").getByRole("button", { name: "Close" }).click();

    // Card drawer: a finished background job with a summary, a PR and a run log.
    await page.getByText("Fix the crash on empty transcript files", { exact: true }).click();
    await page.locator(".drawer .runlog li").first().waitFor();
    await shot(page, "drawer");
    await page.keyboard.press("Escape");

    // A session that waits for a decision.
    await page.getByText("Migrate the auth middleware to the new token format", { exact: true }).click();
    await page.locator(".drawer").waitFor();
    await shot(page, "decision");
    await page.keyboard.press("Escape");

    // A card click scrolls the board. Start the remaining views from the left edge.
    const scrollHome = () => page.evaluate(() => document.querySelector(".board")?.scrollTo(0, 0));
    await scrollHome();

    // The queue strip: what the planner would run next, and when.
    await page.locator(".qhead").click();
    await page.locator(".qsteps li").first().waitFor();
    await shot(page, "queue");
    await page.locator(".qhead").click();

    // Search narrows the board.
    await page.getByPlaceholder("Search title, prompt, project…").fill("test");
    await scrollHome();
    await shot(page, "search");
    await page.getByPlaceholder("Search title, prompt, project…").fill("");

    // Dialogs.
    await page.getByRole("button", { name: "+ Task" }).click();
    await page.getByRole("dialog", { name: "New task" }).waitFor();
    await page.getByPlaceholder("What needs to happen").fill("Add a health endpoint to the API");
    await page
      .getByPlaceholder(/The title is always the first line/)
      .fill("Add GET /health that returns the build version and the database status. Add a test.");
    await shot(page, "new-task");
    // A filled form asks before it closes, so the first Escape shows the bar
    // and the second one discards the draft.
    await page.keyboard.press("Escape");
    await page.locator(".unsaved").waitFor();
    await shot(page, "unsaved");
    await page.keyboard.press("Escape");
    await page.getByRole("dialog", { name: "New task" }).waitFor({ state: "detached" });

    await page.getByRole("button", { name: "Columns" }).click();
    await page.getByRole("dialog", { name: "Columns" }).waitFor();
    await shot(page, "columns");
    await page.keyboard.press("Escape");

    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("dialog", { name: "Settings" }).waitFor();
    await shot(page, "settings");
    await page.keyboard.press("Escape");

    await context.close();
  } finally {
    await browser.close();
    stop();
  }

  writeFileSync(join(OUT, "VERSION"), `${version}\n`);
  console.log(`screenshots for ${version} written to ${OUT}/`);
}

main().catch((e) => {
  console.error(e);
  stop();
  process.exit(1);
});
