import { describe, expect, it } from "vitest";

import { findHardcoded } from "./hardcoded";

const sources = import.meta.glob<string>(["../**/*.tsx", "!../**/*.test.tsx"], {
  query: "?raw",
  import: "default",
  eager: true,
});

describe("hardcoded UI strings", () => {
  it("scanner flags JSX text and literal attributes", () => {
    const src = `
      const a = <p>Hello world</p>;
      const b = <input placeholder="Type here" />;
      const c = <p>{t("ok")}</p>;
      const d = <span aria-hidden>·</span>;
      // <p>commented out</p>
      const e = items.filter((x) => x.a > 1 && x.b < 2);
    `;
    expect(findHardcoded(src).map((f) => f.text)).toEqual(["Hello world", 'placeholder="Type here"']);
  });

  it("scans the app sources", () => {
    expect(Object.keys(sources).length).toBeGreaterThan(30);
  });

  it("no hardcoded strings in components (use i18n keys)", () => {
    const found = Object.entries(sources).flatMap(([file, src]) =>
      findHardcoded(src).map((f) => `${file}:${f.line} ${f.text}`),
    );
    expect(found).toEqual([]);
  });
});
