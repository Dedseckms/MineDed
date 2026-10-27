// Posts CHANGELOG.md sections to the Telegram channel.
// Usage: node scripts/telegram-changelog.mjs <version|all> [--dry-run]
// Env: TELEGRAM_BOT_TOKEN, TELEGRAM_CHAT_ID (falls back to ./.telegram.env).
import { readFileSync, existsSync } from "node:fs";

const REPO = "https://github.com/Dedseckms/MineDed";
const SITE = "https://mineded.dedseckms.ru";

function env() {
  const vars = { ...process.env };
  if ((!vars.TELEGRAM_BOT_TOKEN || !vars.TELEGRAM_CHAT_ID) && existsSync(".telegram.env")) {
    for (const line of readFileSync(".telegram.env", "utf8").split(/\r?\n/)) {
      const m = line.match(/^\s*([A-Z_]+)\s*=\s*(.*)\s*$/);
      if (m && !vars[m[1]]) vars[m[1]] = m[2];
    }
  }
  if (!vars.TELEGRAM_BOT_TOKEN || !vars.TELEGRAM_CHAT_ID) throw new Error("TELEGRAM_BOT_TOKEN / TELEGRAM_CHAT_ID not set");
  return vars;
}

/** Sections in file order (newest first): { version, title, items }. */
function sections() {
  const out = [];
  let cur = null;
  for (const line of readFileSync("CHANGELOG.md", "utf8").split(/\r?\n/)) {
    const h = line.match(/^##\s+(\d+\.\d+\.\d+)\s*[—-]\s*(.+)$/);
    if (h) {
      cur = { version: h[1], title: h[2].trim(), items: [] };
      out.push(cur);
    } else if (cur && /^\s*-\s+/.test(line)) {
      cur.items.push(line.replace(/^\s*-\s+/, "").trim());
    }
  }
  return out;
}

const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const inline = (s) => esc(s).replace(/`([^`]+)`/g, "<code>$1</code>");

function message(s) {
  const body = s.items.map((i) => inline(i)).join("\n\n");
  return (
    `<b>MineDed ${s.version} — ${inline(s.title)}</b>\n\n${body}\n\n` +
    `⬇️ <a href="${REPO}/releases/tag/v${s.version}">Скачать</a> · 🌐 <a href="${SITE}">Сайт</a>\n` +
    `Установленный лаунчер обновится сам.`
  );
}

/** Cover image rendered by docs/changelog/make-cards.ps1, if it was committed. */
async function cardUrl(version) {
  const url = `https://raw.githubusercontent.com/Dedseckms/MineDed/main/docs/changelog/${version}.png`;
  try {
    return (await fetch(url, { method: "HEAD" })).ok ? url : null;
  } catch {
    return null;
  }
}

async function send(vars, text, photo) {
  const method = photo ? "sendPhoto" : "sendMessage";
  const payload = photo
    ? { chat_id: vars.TELEGRAM_CHAT_ID, photo, caption: text, parse_mode: "HTML" }
    : { chat_id: vars.TELEGRAM_CHAT_ID, text, parse_mode: "HTML", disable_web_page_preview: true };
  const res = await fetch(`https://api.telegram.org/bot${vars.TELEGRAM_BOT_TOKEN}/${method}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(payload),
  });
  const json = await res.json();
  if (!json.ok) throw new Error(`telegram: ${json.description}`);
}

const [arg = "", flag] = process.argv.slice(2);
const dry = flag === "--dry-run";
const all = sections();
const picked = arg === "all" ? [...all].reverse() : all.filter((s) => s.version === arg.replace(/^v/, ""));
if (picked.length === 0) {
  console.log(`no CHANGELOG section for ${arg}; nothing posted`);
  process.exit(0);
}
const vars = dry ? null : env();
for (const s of picked) {
  const text = message(s);
  if (dry) console.log(text + "\n---");
  else {
    await send(vars, text, await cardUrl(s.version));
    console.log(`posted ${s.version}`);
    await new Promise((r) => setTimeout(r, 1500));
  }
}
