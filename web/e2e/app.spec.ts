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

test("complete and undo", async ({ page, context }) => {
  const name = "Complete Synthetic task 3";
  // A second page of the same device sees what the daemon holds, not this page's local state.
  const other = await context.newPage();
  await other.goto(base());
  await expect(other.getByRole("checkbox", { name })).toBeVisible();

  await page.getByRole("checkbox", { name }).click();
  await expect(page.getByRole("status")).toContainText('Completed "Synthetic task 3"');
  await expect(page.getByRole("checkbox", { name })).toBeChecked();
  // The completion reached the daemon: a fresh load lists the task as done.
  await other.reload();
  const done = other.locator("section", { has: other.getByRole("heading", { name: "DONE" }) });
  const finished = done.getByRole("checkbox", { name });
  await expect(finished).toBeChecked();
  await expect(finished).toBeDisabled();

  await page.getByRole("button", { name: "Undo" }).click();
  await expect(page.getByRole("status")).toHaveCount(0);
  await other.reload();
  const back = other.getByRole("checkbox", { name });
  await expect(back).toBeVisible();
  await expect(back).not.toBeChecked();
  await expect(back).toBeEnabled();
  await expect(other.getByRole("heading", { name: "DONE" })).toHaveCount(0);
  await page.reload();
  await expect(page.getByRole("checkbox", { name })).not.toBeChecked();
  await expect(page.getByRole("heading", { name: "DONE" })).toHaveCount(0);
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

test("tick and untick a checklist item", async ({ page }, info) => {
  await page.getByRole("button", { name: /Synthetic packing/ }).click();
  await expect(page.getByRole("heading", { name: "Synthetic packing" })).toBeVisible();
  await expect(page.getByText("Items · 2 open")).toBeVisible();
  if (process.env.E2E_SCREENSHOTS) {
    await page.screenshot({
      path: `${process.env.E2E_SCREENSHOTS}/task-${info.project.name}.png`,
    });
  }
  const socks = (verb: string) => page.getByRole("checkbox", { name: `${verb} Synthetic socks` });
  await socks("Complete").click();
  await expect(page.getByRole("status")).toContainText('Completed "Synthetic socks"');
  await page.reload();
  await expect(socks("Reopen")).toBeChecked();
  await socks("Reopen").click();
  await expect(page.getByRole("status")).toContainText('Reopened "Synthetic socks"');
  await expect(socks("Complete")).not.toBeChecked();
  await socks("Complete").click();
  await expect(page.getByRole("status")).toContainText('Completed "Synthetic socks"');
  await page.getByRole("button", { name: "Undo" }).click();
  await expect(socks("Complete")).not.toBeChecked();
  await page.goBack();
  await expect(page.getByRole("heading", { name: "Inbox" })).toBeVisible();
});

test("new task sheet slides up full screen", async ({ page }, info) => {
  await expect(page.getByRole("checkbox", { name: "Complete Synthetic task 1" })).toBeVisible();
  await page.getByRole("button", { name: "New task" }).click();
  const form = page.getByRole("form", { name: "New task" });
  await expect(form).toBeVisible();
  // A fixed inset-0 box is viewport-sized even mid-slide: wait until it has also arrived at the
  // top-left corner, i.e. the slide-up has finished.
  const view = page.viewportSize()!;
  await expect
    .poll(async () => {
      const b = await form.boundingBox();
      if (!b) return false;
      return (
        b.y <= 1 &&
        Math.abs(b.x) <= 1 &&
        Math.abs(b.width - view.width) <= 1 &&
        Math.abs(b.height - view.height) <= 1
      );
    })
    .toBe(true);
  if (process.env.E2E_SCREENSHOTS) {
    await page.screenshot({
      path: `${process.env.E2E_SCREENSHOTS}/sheet-${info.project.name}.png`,
    });
  }
  await page.getByLabel("Title").fill("Synthetic captured");
  await page.getByRole("button", { name: "Add" }).click();
  await expect(page.getByRole("status")).toContainText('Added "Synthetic captured"');
});

/**
 * Records each screen slide (#157) as its direction and the heading on screen when the browser
 * has taken the outgoing snapshot, which must still be the screen being left. Null in a browser
 * without view transitions.
 */
async function recordSlides(page: Page): Promise<(() => Promise<string[]>) | null> {
  const has = await page.evaluate(() => {
    const d = document as Document & { startViewTransition?: (f: () => unknown) => unknown };
    const start = d.startViewTransition;
    if (!start) return false;
    const w = window as unknown as { slides: string[] };
    w.slides = [];
    d.startViewTransition = (f) => {
      const dir = document.documentElement.dataset.nav ?? "";
      return start.call(d, () => {
        w.slides.push(`${dir}:${document.querySelector("h1")?.textContent ?? ""}`);
        return f();
      });
    };
    return true;
  });
  return has ? () => page.evaluate(() => (window as unknown as { slides: string[] }).slides) : null;
}

test("a task screen slides in and back out", async ({ page }, info) => {
  const title = `Synthetic trip ${info.project.name}`;
  await expect(page.getByRole("heading", { name: "Inbox" })).toBeVisible();
  const slides = await recordSlides(page);
  await page.getByRole("button", { name: new RegExp(title) }).click();
  await expect(page.getByRole("heading", { name: title })).toBeVisible();
  await page.getByRole("button", { name: "Inbox" }).click();
  await expect(page.getByRole("heading", { name: "Inbox" })).toBeVisible();
  // The browser's own back slides the same way.
  await page.getByRole("button", { name: new RegExp(title) }).click();
  await expect(page.getByRole("heading", { name: title })).toBeVisible();
  await page.goBack();
  await expect(page.getByRole("heading", { name: "Inbox" })).toBeVisible();
  if (slides) {
    const pair = ["forward:inbox", `back:${title}`];
    await expect.poll(slides).toEqual([...pair, ...pair]);
  }
});

test.describe("phone editing", () => {
  const open = (page: Page, title: string) =>
    page.getByRole("button", { name: new RegExp(title) }).click();
  const save = async (page: Page, name: string) => {
    // Save waits for the debounced form check to return the write.
    const button = page.getByRole("button", { name, exact: true });
    await expect(button).toBeEnabled();
    await button.click();
  };

  test("edit a due date and a repeat", async ({ page }, info) => {
    const title = `Synthetic edit ${info.project.name}`;
    await open(page, title);
    await expect(page.getByRole("heading", { name: title })).toBeVisible();
    await page.getByRole("button", { name: "Edit" }).click();
    const form = page.getByRole("form", { name: "Edit" });
    await form.getByLabel("Due", { exact: true }).fill("tomorrow");
    await expect(form).toContainText("→");
    await form.getByLabel("Repeat").fill("weekly");
    await expect(form).toContainText("→ Weekly");
    await save(page, "Save");
    await expect(page.getByRole("status")).toContainText(`Saved "${title}"`);
    await expect(page.locator("dl.facts dt", { hasText: "Due" })).toBeVisible();
    await expect(page.locator("dl.facts dt", { hasText: "Repeat" })).toBeVisible();
  });

  test("add an item", async ({ page }, info) => {
    await open(page, `Synthetic trip ${info.project.name}`);
    await page.getByRole("button", { name: "+ Add item" }).click();
    const form = page.getByRole("form", { name: "New item" });
    await form.getByLabel("Title").fill("Synthetic towel");
    await save(page, "Add");
    await expect(page.getByRole("status")).toContainText('Added "Synthetic towel"');
    await expect(page.getByRole("checkbox", { name: "Complete Synthetic towel" })).toBeVisible();
  });

  test("delete with undo", async ({ page }, info) => {
    const title = `Synthetic delete ${info.project.name}`;
    const row = page.getByRole("checkbox", { name: `Complete ${title}` });
    await open(page, title);
    await page.getByRole("button", { name: "Delete task" }).click();
    await page.getByRole("button", { name: "Delete", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Inbox" })).toBeVisible();
    await expect(page.getByRole("status")).toContainText(`Deleted "${title}"`);
    await expect(row).toHaveCount(0);
    await page.getByRole("button", { name: "Undo" }).click();
    await expect(row).toBeVisible();
  });

  test("create a task with a list and a due date", async ({ page }, info) => {
    const title = `Synthetic created ${info.project.name}`;
    await expect(page.getByRole("checkbox", { name: "Complete Synthetic task 1" })).toBeVisible();
    await page.getByRole("button", { name: "New task" }).click();
    const form = page.getByRole("form", { name: "New task" });
    await form.getByLabel("Title").fill(title);
    await form.getByLabel("List").selectOption({ label: "errands" });
    await form.getByLabel("Due", { exact: true }).fill("today");
    await save(page, "Add");
    await expect(page.getByRole("status")).toContainText(`Added "${title}"`);
    await page.getByRole("tab", { name: "Today" }).click();
    await expect(page.getByRole("checkbox", { name: `Complete ${title}` })).toBeVisible();
  });
});

test.describe("views tab", () => {
  // The daemon is shared with the other specs and the other colour scheme: leave it as found.
  test.afterEach(async ({ page }) => {
    const undo = page.getByRole("button", { name: "Undo" });
    if (await undo.isVisible()) await undo.click();
  });

  test("opens a list and completes in it", async ({ page }) => {
    const work = () => page.getByRole("button", { name: /^work/ });
    await page.getByRole("tab", { name: "Views" }).click();
    await expect(page.getByRole("heading", { name: "Views" })).toBeVisible();
    for (const name of ["Inbox", "Today", "All"]) {
      await expect(page.getByRole("button", { name })).toBeVisible();
    }
    await expect(work()).toBeVisible();
    // The list's two tasks are due today: one count for due, one for open.
    await expect(work().locator(".view-due")).toHaveText("2");
    await expect(work().locator(".view-total")).toHaveText("2");
    const slides = await recordSlides(page);
    await work().click();
    await expect(page.getByRole("heading", { name: "work" })).toBeVisible();

    await page.getByRole("checkbox", { name: "Complete Synthetic due today" }).click();
    await expect(page.getByRole("status")).toContainText('Completed "Synthetic due today"');

    await page.getByRole("button", { name: "Views" }).click();
    await expect(page.getByRole("heading", { name: "Views" })).toBeVisible();
    await expect(work().locator(".view-due")).toHaveText("1");
    await expect(work().locator(".view-total")).toHaveText("1");
    if (slides) expect(await slides()).toEqual(["forward:Views", "back:work"]);
  });
});

test("settings opens from the views screen", async ({ page }) => {
  await page.getByRole("tab", { name: "Views" }).click();
  await expect(page.getByRole("heading", { name: "Views" })).toBeVisible();
  await page.getByRole("button", { name: "Settings" }).click();
  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
  const about = page.locator("section", { has: page.getByRole("heading", { name: "About" }) });
  await expect(about.getByText(/^\d+\.\d+\.\d+/)).toBeVisible();
  await expect(about.getByText("Protocol")).toBeVisible();
  await page.getByRole("button", { name: "Views" }).click();
  await expect(page.getByRole("heading", { name: "Views" })).toBeVisible();
});

// Last: it stops the daemon, and restarts it for whatever runs next.
test("daemon stopped shows offline state", async ({ page }) => {
  test.setTimeout(60_000);
  stopDaemon();
  try {
    await expect(page.getByText("Can't reach Twydo")).toBeVisible({ timeout: 20_000 });
    await expect(page.getByRole("button", { name: "New task" })).toHaveCount(0);
  } finally {
    startDaemon();
    await waitForDaemon();
  }
});
