import { describe, expect, it } from "vitest";

import { ERROR_CODES, TASK_KINDS } from "../lib/errors";
import en from "./en.json";
import ru from "./ru.json";

const PLURAL = /_(zero|one|two|few|many|other)$/;

/** Flattened keys; plural forms collapse to their base key (RU has more forms than EN). */
function keys(obj: object, prefix = ""): Set<string> {
  const out = new Set<string>();
  for (const [k, v] of Object.entries(obj)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (v && typeof v === "object") keys(v, key).forEach((x) => out.add(x));
    else out.add(key.replace(PLURAL, ""));
  }
  return out;
}

function get(obj: unknown, path: string): unknown {
  return path.split(".").reduce<unknown>((o, k) => (o && typeof o === "object" ? (o as Record<string, unknown>)[k] : undefined), obj);
}

describe("i18n", () => {
  const ruKeys = keys(ru);
  const enKeys = keys(en);

  it("ru and en have the same keys", () => {
    expect([...ruKeys].filter((k) => !enKeys.has(k))).toEqual([]);
    expect([...enKeys].filter((k) => !ruKeys.has(k))).toEqual([]);
  });

  it("has a message for every ErrorCode", () => {
    for (const code of ERROR_CODES) {
      expect(typeof get(ru, `errors.${code}`), `ru errors.${code}`).toBe("string");
      expect(typeof get(en, `errors.${code}`), `en errors.${code}`).toBe("string");
    }
  });

  it("has a title for every TaskKind", () => {
    for (const kind of TASK_KINDS) {
      expect(typeof get(ru, `tasks.${kind}`)).toBe("string");
      expect(typeof get(en, `tasks.${kind}`)).toBe("string");
    }
  });

  it("no empty strings", () => {
    const empty = (o: object, p = ""): string[] =>
      Object.entries(o).flatMap(([k, v]) =>
        v && typeof v === "object" ? empty(v, `${p}${k}.`) : v === "" ? [`${p}${k}`] : [],
      );
    expect(empty(ru)).toEqual([]);
    expect(empty(en)).toEqual([]);
  });
});
