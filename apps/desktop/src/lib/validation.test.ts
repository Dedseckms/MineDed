import { describe, expect, it } from "vitest";

import {
  type ProxyDraft,
  draftToProxy,
  formatArgs,
  hasErrors,
  isValidHost,
  parseArgs,
  proxyToDraft,
  validateInstanceName,
  validateMemory,
  validateNick,
  validateOptionalPath,
  validateProxy,
} from "./validation";

describe("validateNick", () => {
  it.each(["Steve", "abc", "A_1", "x".repeat(16), "Player_2024"])("accepts %s", (n) => {
    expect(validateNick(n)).toBeNull();
  });
  it("requires a value", () => expect(validateNick("")).toBe("validation.required"));
  it.each(["ab", "x".repeat(17)])("rejects length of %s", (n) => {
    expect(validateNick(n)).toBe("validation.nick.length");
  });
  it.each(["Стив", "with space", "dash-name", "dot.name", "emoji😀", "tab\tx"])("rejects chars in %s", (n) => {
    expect(validateNick(n)).toBe("validation.nick.chars");
  });
});

const proxy = (p: Partial<ProxyDraft>): ProxyDraft => ({
  mode: "http",
  host: "127.0.0.1",
  port: "8080",
  username: "",
  password: "",
  ...p,
});

describe("validateProxy", () => {
  it("off mode is always valid", () => {
    expect(validateProxy(proxy({ mode: "off", host: "", port: "x" }))).toEqual({});
  });
  it("accepts valid http and socks5", () => {
    expect(validateProxy(proxy({}))).toEqual({});
    expect(validateProxy(proxy({ mode: "socks5", host: "proxy.example.com", port: "1080", username: "u", password: "p" }))).toEqual({});
    expect(validateProxy(proxy({ host: "::1" }))).toEqual({});
  });
  it("requires host and port", () => {
    expect(validateProxy(proxy({ host: " ", port: "" }))).toEqual({
      host: "validation.required",
      port: "validation.required",
    });
  });
  it("rejects schemes and bad hosts", () => {
    expect(validateProxy(proxy({ host: "http://proxy" })).host).toBe("validation.proxy.scheme");
    expect(validateProxy(proxy({ host: "bad host" })).host).toBe("validation.proxy.host");
    expect(validateProxy(proxy({ host: "256.1.1.1" })).host).toBe("validation.proxy.host");
    expect(validateProxy(proxy({ host: "-bad.com" })).host).toBe("validation.proxy.host");
  });
  it.each(["0", "65536", "80a", "-1", "1.5", "999999"])("rejects port %s", (port) => {
    expect(validateProxy(proxy({ port })).port).toBe("validation.proxy.port");
  });
  it.each(["1", "65535", " 3128 "])("accepts port %s", (port) => {
    expect(validateProxy(proxy({ port })).port).toBeUndefined();
  });
  it("password requires username; credentials are length-limited", () => {
    expect(validateProxy(proxy({ password: "secret" })).username).toBe("validation.proxy.usernameRequired");
    expect(validateProxy(proxy({ username: "u".repeat(256) })).username).toBe("validation.proxy.credLength");
    expect(validateProxy(proxy({ username: "u", password: "p".repeat(256) })).password).toBe(
      "validation.proxy.credLength",
    );
  });
  it("round-trips through the wire type", () => {
    const cfg = draftToProxy(proxy({ mode: "socks5", host: " h.example ", port: "1080", username: "u" }));
    expect(cfg).toEqual({ kind: "socks5", host: "h.example", port: 1080, username: "u", password: null });
    expect(proxyToDraft(cfg)).toEqual(proxy({ mode: "socks5", host: "h.example", port: "1080", username: "u" }));
    expect(draftToProxy(proxyToDraft(null))).toBeNull();
  });
  it("isValidHost", () => {
    expect(isValidHost("localhost")).toBe(true);
    expect(isValidHost("1.2.3")).toBe(false);
    expect(isValidHost("")).toBe(false);
  });
});

describe("other validators", () => {
  it("instance name", () => {
    expect(validateInstanceName("My pack 1.21")).toBeNull();
    expect(validateInstanceName("  ")).toBe("validation.required");
    expect(validateInstanceName("a/b")).toBe("validation.name.chars");
    expect(validateInstanceName("x".repeat(65))).toBe("validation.name.length");
  });
  it("optional path", () => {
    expect(validateOptionalPath("")).toBeNull();
    expect(validateOptionalPath("C:\\Games\\MC")).toBeNull();
    expect(validateOptionalPath("/home/u/.mineup")).toBeNull();
    expect(validateOptionalPath("games")).toBe("validation.path.absolute");
  });
  it("memory", () => {
    expect(validateMemory(1024, 4096, 16384)).toEqual({});
    expect(validateMemory(256, 4096, null).minMb).toBe("validation.memory.minTooLow");
    expect(validateMemory(4096, 2048, null).maxMb).toBe("validation.memory.maxBelowMin");
    expect(validateMemory(1024, 32768, 16384).maxMb).toBe("validation.memory.exceedsRam");
    expect(hasErrors(validateMemory(1024, 2048, null))).toBe(false);
  });
  it("jvm args", () => {
    expect(parseArgs(`-Xss2m  -Dfoo="a b" '-Dbar=c d' ""`)).toEqual({
      ok: true,
      args: ["-Xss2m", "-Dfoo=a b", "-Dbar=c d", ""],
    });
    expect(parseArgs(`-Dfoo="open`)).toEqual({ ok: false, error: "validation.args.quote" });
    const args = ["-XX:+UseG1GC", "-Dx=a b"];
    const parsed = parseArgs(formatArgs(args));
    expect(parsed).toEqual({ ok: true, args });
  });
});
