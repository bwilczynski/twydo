import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { committed, navigate } from "./navigate";

type Start = (update: () => Promise<void>) => { finished: Promise<void> };
const doc = document as unknown as { startViewTransition?: Start };

let reduced = false;
beforeEach(() => {
  reduced = false;
  vi.stubGlobal("matchMedia", (q: string) => ({ matches: q.includes("reduce") && reduced }));
});
afterEach(() => {
  delete doc.startViewTransition;
  delete document.documentElement.dataset.nav;
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

/** A fake transition: runs the update at once and finishes when it settles. */
function fake() {
  const calls: { done: Promise<void> }[] = [];
  doc.startViewTransition = (update) => {
    const done = update();
    calls.push({ done });
    return { finished: done.then(() => undefined) };
  };
  return calls;
}

describe("navigate", () => {
  it("updates at once where view transitions are missing", () => {
    const update = vi.fn();
    navigate("forward", update);
    expect(update).toHaveBeenCalledOnce();
    expect(document.documentElement.dataset.nav).toBeUndefined();
  });

  it("updates at once when the phone asks for reduced motion", () => {
    const calls = fake();
    reduced = true;
    const update = vi.fn();
    navigate("back", update);
    expect(update).toHaveBeenCalledOnce();
    expect(calls).toHaveLength(0);
  });

  it("marks the direction and holds the snapshot until the render commits", async () => {
    const calls = fake();
    const update = vi.fn();
    navigate("back", update);
    expect(update).toHaveBeenCalledOnce();
    expect(document.documentElement.dataset.nav).toBe("back");
    let settled = false;
    calls[0]!.done.then(() => (settled = true));
    await Promise.resolve();
    expect(settled).toBe(false);
    committed();
    await calls[0]!.done;
    await Promise.resolve();
    expect(document.documentElement.dataset.nav).toBeUndefined();
  });

  it("keeps the newer direction when an older transition ends", async () => {
    const calls = fake();
    navigate("forward", () => {});
    committed();
    navigate("forward", () => {});
    await calls[0]!.done;
    await Promise.resolve();
    expect(document.documentElement.dataset.nav).toBe("forward");
  });

  it("lets the snapshot go when no render follows", async () => {
    vi.useFakeTimers();
    const calls = fake();
    navigate("forward", () => {});
    let settled = false;
    calls[0]!.done.then(() => (settled = true));
    await vi.advanceTimersByTimeAsync(200);
    expect(settled).toBe(true);
  });
});
