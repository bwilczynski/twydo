import { describe, expect, it } from "vitest";
import { parseTaskHash, taskHash } from "./route";

describe("task route", () => {
  it("round trips a uid with slash, at sign and spaces", () => {
    const uid = "a/b@c d";
    expect(taskHash(uid)).toBe("#/task/a%2Fb%40c%20d");
    expect(parseTaskHash(taskHash(uid))).toBe(uid);
  });

  it("anything else is not a task", () => {
    expect(parseTaskHash("")).toBeUndefined();
    expect(parseTaskHash("#/")).toBeUndefined();
    expect(parseTaskHash("#/task/")).toBeUndefined();
    expect(parseTaskHash("#/today")).toBeUndefined();
  });
});
