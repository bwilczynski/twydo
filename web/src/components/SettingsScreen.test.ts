import { describe, expect, it } from "vitest";
import { lastSynced } from "./SettingsScreen";

describe("lastSynced", () => {
  const now = new Date(2026, 10, 23, 15, 0);

  it("is Never without a success", () => {
    expect(lastSynced(null, now)).toBe("Never");
    expect(lastSynced(undefined, now)).toBe("Never");
    expect(lastSynced("garbage", now)).toBe("Never");
  });

  it("shows only the local time today", () => {
    expect(lastSynced(new Date(2026, 10, 23, 9, 5).toISOString(), now)).toBe("09:05");
  });

  it("adds the day and month on another day", () => {
    expect(lastSynced(new Date(2026, 10, 21, 22, 41).toISOString(), now)).toBe("21 Nov 22:41");
  });
});
