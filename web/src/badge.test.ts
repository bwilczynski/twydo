import { describe, expect, it, vi } from "vitest";
import { setBadge, type BadgeNav } from "./badge";

const nav = (mode: "ok" | "throws" | "rejects" = "ok") => {
  const f = () => {
    if (mode === "throws") throw new Error("no");
    return mode === "rejects" ? Promise.reject(new Error("no")) : Promise.resolve();
  };
  return { setAppBadge: vi.fn(f), clearAppBadge: vi.fn(f) };
};

describe("setBadge", () => {
  it("unknown count leaves the badge", () => {
    const n = nav();
    setBadge(undefined, n);
    expect(n.setAppBadge).not.toHaveBeenCalled();
    expect(n.clearAppBadge).not.toHaveBeenCalled();
  });

  it("zero clears the badge", () => {
    const n = nav();
    setBadge(0, n);
    expect(n.clearAppBadge).toHaveBeenCalledTimes(1);
    expect(n.setAppBadge).not.toHaveBeenCalled();
  });

  it("a count sets the badge", () => {
    const n = nav();
    setBadge(5, n);
    expect(n.setAppBadge).toHaveBeenCalledWith(5);
  });

  it("missing API does nothing", () => {
    expect(() => setBadge(5, {} as BadgeNav)).not.toThrow();
    expect(() => setBadge(0, {} as BadgeNav)).not.toThrow();
  });

  it("a throwing API is swallowed", () => {
    expect(() => setBadge(3, nav("throws"))).not.toThrow();
    expect(() => setBadge(0, nav("throws"))).not.toThrow();
  });

  it("a rejecting API gets a rejection handler", async () => {
    for (const n of [3, 0]) {
      const rejected = Promise.reject(new Error("no"));
      const handled = vi.spyOn(rejected, "catch");
      const then = vi.spyOn(rejected, "then");
      setBadge(n, { setAppBadge: () => rejected, clearAppBadge: () => rejected });
      const viaThen = then.mock.calls.some((c) => typeof c[1] === "function");
      expect(handled.mock.calls.length > 0 || viaThen).toBe(true);
      await rejected.catch(() => {});
    }
  });
});
