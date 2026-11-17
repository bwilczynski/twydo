import { expect, type Page, test } from "@playwright/test";
import { pairingCode, startDaemon, stopDaemon, waitForDaemon } from "./setup";

let paired = 0;
const base = () => process.env.E2E_BASE_URL!;

async function pair(page: Page, name: string) {
  await page.goto(base());
  await page.getByLabel("Pairing code").fill(pairingCode(name));
  await page.getByRole("button", { name: "Pair" }).click();
  await expect(page.getByRole("tab", { name: "Inbox" })).toBeVisible();
}

test.beforeEach(async ({ page }, info) => {
  await pair(page, `e2e-${info.project.name}-${++paired}-${Date.now() % 1e6}`);
});

test("pairing then Inbox renders", async ({ page }) => {
  await expect(page.getByRole("heading", { name: "Inbox" })).toBeVisible();
  await expect(page.getByRole("checkbox", { name: "Complete Synthetic task 1" })).toBeVisible();
  await expect(page.getByRole("checkbox", { name: "Complete Synthetic task 2" })).toBeVisible();
});

test("Today lists the due tasks", async ({ page }, info) => {
  await page.getByRole("tab", { name: "Today" }).click();
  await expect(page.getByRole("heading", { name: "Today" })).toBeVisible();
  await expect(page.getByRole("checkbox", { name: "Complete Synthetic due today" })).toBeVisible();
  await expect(
    page.getByRole("checkbox", { name: "Complete Synthetic also due today" }),
  ).toBeVisible();
  if (process.env.E2E_SCREENSHOTS) {
    await page.screenshot({
      path: `${process.env.E2E_SCREENSHOTS}/today-${info.project.name}.png`,
    });
  }
});

test("complete and undo", async ({ page }) => {
  const box = page.getByRole("checkbox", { name: "Complete Synthetic task 3" });
  await box.click();
  await expect(page.getByRole("status")).toContainText('Completed "Synthetic task 3"');
  await page.getByRole("button", { name: "Undo" }).click();
  await expect(page.getByRole("status")).toHaveCount(0);
  await page.reload();
  await expect(page.getByRole("checkbox", { name: "Complete Synthetic task 3" })).toBeEnabled();
});

test("capture from Inbox and from Today", async ({ page }, info) => {
  const tag = info.project.name;
  const add = async (title: string) => {
    await page.getByRole("button", { name: "New task" }).click();
    await page.getByLabel("Title").fill(title);
    await page.getByRole("button", { name: "Add" }).click();
    await expect(page.getByRole("status")).toContainText(`Added "${title}"`);
  };
  // The capture target arrives with the view: wait for rows before opening the sheet.
  await expect(page.getByRole("checkbox", { name: "Complete Synthetic task 1" })).toBeVisible();
  await add(`Captured in Inbox ${tag}`);
  await expect(
    page.getByRole("checkbox", { name: `Complete Captured in Inbox ${tag}` }),
  ).toBeVisible();

  await page.getByRole("tab", { name: "Today" }).click();
  await expect(page.getByRole("heading", { name: "Today" })).toBeVisible();
  await expect(page.getByRole("checkbox", { name: "Complete Synthetic due today" })).toBeVisible();
  await add(`Captured in Today ${tag}`);
  await expect(
    page.getByRole("checkbox", { name: `Complete Captured in Today ${tag}` }),
  ).toBeVisible();
});

// Last: it stops the daemon, and restarts it for whatever runs next.
test("daemon stopped shows offline state", async ({ page }) => {
  stopDaemon();
  try {
    await expect(page.getByText("Can't reach Twydo")).toBeVisible({ timeout: 20_000 });
    await expect(page.getByRole("button", { name: "New task" })).toHaveCount(0);
  } finally {
    startDaemon();
    await waitForDaemon();
  }
});
