import { describe, expect, it } from "vitest";

import type { LocalDepIssue } from "../bindings";
import { groupIssuesByFile, suggestedPicks } from "./content";
import { extractInviteCode, parseDeepLink } from "./deepLink";
import { validateRegister } from "./validation";

describe("parseDeepLink", () => {
  it("knows open / join / invite for both schemes", () => {
    expect(parseDeepLink("mineded://open")).toEqual({ kind: "open" });
    expect(parseDeepLink("mineup://open/")).toEqual({ kind: "open" });
    expect(parseDeepLink("MINEDED://join/ABCD-1234")).toEqual({ kind: "join", code: "ABCD-1234" });
    expect(parseDeepLink("mineup://join/abcd-1234/?x=1")).toEqual({ kind: "join", code: "abcd-1234" });
    expect(parseDeepLink("mineded://invite/Xy_9-z")).toEqual({ kind: "invite", code: "Xy_9-z" });
  });

  it("rejects other schemes, actions and odd codes", () => {
    expect(parseDeepLink("https://example.com/join/ABCD")).toBeNull();
    expect(parseDeepLink("mineded://join/")).toBeNull();
    expect(parseDeepLink("mineded://join/a%20b")).toBeNull();
    expect(parseDeepLink("mineded://join/%E0%A4%A")).toBeNull();
    expect(parseDeepLink("mineded://delete/all")).toBeNull();
    expect(parseDeepLink("not a link")).toBeNull();
  });
});

describe("extractInviteCode", () => {
  it("accepts a raw code, a Drasl invite URL or a deep link", () => {
    expect(extractInviteCode("  abc123  ")).toBe("abc123");
    expect(extractInviteCode("https://skin.example/web/registration?invite=Inv_42")).toBe("Inv_42");
    expect(extractInviteCode("http://h/web/registration?foo=1&invite=xyz9")).toBe("xyz9");
    expect(extractInviteCode("mineded://invite/code-77")).toBe("code-77");
    // URL without the parameter stays as is (and fails validation).
    expect(extractInviteCode("https://skin.example/web/registration")).toBe("https://skin.example/web/registration");
  });

  it("registration validation uses the extracted code", () => {
    const d = { invite: "https://s/web/registration?invite=abcd1234", username: "Steve", password: "password1", confirm: "password1" };
    expect(validateRegister(d).invite).toBeUndefined();
    expect(validateRegister({ ...d, invite: "https://s/web/registration" }).invite).toBe("validation.invite");
  });
});

describe("local dependency issues", () => {
  const issue = (fileName: string, projectId: string | null, kind: LocalDepIssue["kind"] = "missing"): LocalDepIssue => ({
    fileName,
    kind,
    modId: projectId ?? "x",
    versionReq: null,
    suggestion: projectId ? { source: "modrinth", projectId, versionId: null } : null,
    suggestionTitle: projectId ? `T-${projectId}` : null,
  });

  it("groups by file in input order", () => {
    const g = groupIssuesByFile([issue("a.jar", "p1"), issue("b.jar", null, "conflict"), issue("a.jar", "p2")]);
    expect(g.map(([f, l]) => [f, l.length])).toEqual([
      ["a.jar", 2],
      ["b.jar", 1],
    ]);
  });

  it("dedupes suggestions by project and keeps titles", () => {
    const { picks, titles } = suggestedPicks([issue("a.jar", "p1"), issue("b.jar", "p1"), issue("c.jar", null), issue("d.jar", "p2")]);
    expect(picks.map((p) => p.projectId)).toEqual(["p1", "p2"]);
    expect(titles).toEqual({ p1: "T-p1", p2: "T-p2" });
  });
});
