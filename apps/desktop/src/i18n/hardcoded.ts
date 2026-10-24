/**
 * Heuristic scanner for hardcoded UI strings in TSX sources (used by tests).
 * Flags JSX text with letters and literal user-facing attributes.
 */

export interface Finding {
  line: number;
  text: string;
}

const ATTRS = ["aria-label", "title", "placeholder", "alt", "label", "description"];
const LETTER = /\p{L}{2,}/u;

/** Removes comments while keeping line numbers (strings with `//` such as URLs survive). */
function stripComments(src: string): string {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " "))
    .replace(/(^|[\s;{}(),])\/\/[^\n]*/g, (m, p: string) => p + " ".repeat(m.length - p.length));
}

function lineOf(src: string, index: number): number {
  let n = 1;
  for (let i = 0; i < index; i++) if (src.charCodeAt(i) === 10) n++;
  return n;
}

export function findHardcoded(source: string): Finding[] {
  const src = stripComments(source);
  const out: Finding[] = [];
  // Text between a closing `>` of a JSX tag and the next `<` / `{`.
  const text = /(?<=[\w"'}\/]\s*>)([^<>{}]+)(?=[<{])/g;
  for (const m of src.matchAll(text)) {
    const raw = m[1] ?? "";
    const value = raw.trim();
    if (!value || !LETTER.test(value)) continue;
    // Skip TypeScript that only looks like JSX text (generics, comparisons, arrows).
    if (/[;=()&|?:`]|\bconst\b|\breturn\b/.test(value)) continue;
    out.push({ line: lineOf(src, m.index ?? 0), text: value });
  }
  const attr = new RegExp(`\\s(${ATTRS.join("|")})="([^"]*)"`, "g");
  for (const m of src.matchAll(attr)) {
    const value = m[2] ?? "";
    if (LETTER.test(value)) out.push({ line: lineOf(src, m.index ?? 0), text: `${m[1]}="${value}"` });
  }
  return out;
}
