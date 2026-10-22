import { describe, expect, it } from "vitest";

import type { VpnNode } from "../bindings";
import {
  SUBSCRIPTION_MAX,
  filterNodes,
  latencyTone,
  parseSubscriptionUrls,
  selectNode,
  sortNodes,
  validateSubscriptionPath,
} from "./vpn";

const node = (name: string, latencyMs: number | null, protocol = "vless", selected = false): VpnNode => ({
  name,
  protocol,
  latencyMs,
  selected,
});

describe("parseSubscriptionUrls", () => {
  it("splits lines, spaces and commas and dedupes", () => {
    expect(parseSubscriptionUrls(" https://a.example/sub \n\nhttp://b.example/x, https://a.example/sub ")).toEqual({
      ok: true,
      urls: ["https://a.example/sub", "http://b.example/x"],
    });
  });

  it("rejects empty input, other schemes and garbage", () => {
    expect(parseSubscriptionUrls("  \n ")).toEqual({ ok: false, error: "validation.required" });
    expect(parseSubscriptionUrls("vless://abc@host:443")).toEqual({ ok: false, error: "validation.url.scheme" });
    expect(parseSubscriptionUrls("https://")).toEqual({ ok: false, error: "validation.url.invalid" });
  });

  it("limits the number of links", () => {
    const many = Array.from({ length: SUBSCRIPTION_MAX + 1 }, (_, i) => `https://h${i}.example/`).join("\n");
    expect(parseSubscriptionUrls(many)).toEqual({ ok: false, error: "validation.vpn.tooMany" });
  });
});

describe("validateSubscriptionPath", () => {
  it("accepts absolute paths with any extension", () => {
    expect(validateSubscriptionPath("C:\\Users\\me\\sub.yaml")).toBeNull();
    expect(validateSubscriptionPath("/home/me/sub.txt")).toBeNull();
  });

  it("rejects empty, relative, directory and control-char paths", () => {
    expect(validateSubscriptionPath("")).toBe("validation.required");
    expect(validateSubscriptionPath("sub.yaml")).toBe("validation.path.absolute");
    expect(validateSubscriptionPath("/home/me/")).toBe("validation.path.ext");
    expect(validateSubscriptionPath("/home/\u0001x")).toBe("validation.path.chars");
  });
});

describe("nodes", () => {
  const list = [node("b", 300), node("a", null), node("c", 40, "trojan"), node("d", 40)];

  it("latency tone thresholds", () => {
    expect(latencyTone(null)).toBe("neutral");
    expect(latencyTone(150)).toBe("accent");
    expect(latencyTone(151)).toBe("warning");
    expect(latencyTone(401)).toBe("danger");
  });

  it("sorts by latency with unknown last, ties by name", () => {
    expect(sortNodes(list, "latency").map((n) => n.name)).toEqual(["c", "d", "b", "a"]);
    expect(sortNodes(list, "name").map((n) => n.name)).toEqual(["a", "b", "c", "d"]);
    expect(list.map((n) => n.name)).toEqual(["b", "a", "c", "d"]);
  });

  it("filters by name or protocol", () => {
    expect(filterNodes(list, " TROJ ").map((n) => n.name)).toEqual(["c"]);
    expect(filterNodes(list, "")).toHaveLength(4);
  });

  it("selects exactly one node, keeping untouched objects", () => {
    const before = [node("a", 1, "vless", true), node("b", 2), node("c", 3)];
    const after = selectNode(before, "b");
    expect(after.map((n) => n.selected)).toEqual([false, true, false]);
    expect(after[2]).toBe(before[2]);
  });
});
