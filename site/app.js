// SPDX-License-Identifier: GPL-3.0-or-later
(function () {
  "use strict";
  var REPO = "Dedseckms/MineUP";

  var DICT = {
    en: {
      title: "MineUP — Minecraft launcher",
      skip: "Skip to content",
      navLabel: "Main",
      navFeatures: "Features",
      navScreens: "Screens",
      navDownload: "Download",
      sceneBtn: "Change background",
      langBtn: "Русский",
      logoAlt: "MineUP logo",
      pitch: "A fast, free Minecraft launcher with mods, skins, multiplayer by room code and a built-in VPN.",
      ctaDownload: "Download",
      ctaGithub: "Source on GitHub",
      platforms: "Windows 10/11 and Linux · free and open source (GPL-3.0)",
      featuresTitle: "Everything in one launcher",
      f1t: "Versions & loaders",
      f1d: "Any Minecraft version, plus Fabric, Quilt, Forge and NeoForge in one click. Java is installed automatically.",
      f2t: "Mods & modpacks",
      f2d: "Browse Modrinth right in the app: mods, modpacks, resource packs and shaders with dependencies resolved.",
      f3t: "Own skin server",
      f3d: "Upload skins and capes, see them in 3D, and everyone on the server sees them too. No Microsoft account required.",
      f4t: "P2P multiplayer",
      f4d: "Create a room, share the code, play together. Direct connection with relay fallback, no port forwarding.",
      f5t: "Voice & chat",
      f5d: "Built-in room chat and voice with push-to-talk and an in-game overlay.",
      f6t: "VPN & anti-blocking",
      f6d: "Downloads and services keep working where they are blocked: built-in VPN, mirrors and smart routing.",
      f7t: "Low-end PC mode",
      f7d: "A light mode without heavy effects, plus tuned memory and JVM flags for weak hardware.",
      f8t: "Auto-updates",
      f8d: "The launcher updates itself in the background with signed releases.",
      screensTitle: "How it looks",
      screensNote: "Illustrations of the interface drawn with CSS, not real screenshots.",
      s1: "Instance library",
      s1alt: "Instance library illustration",
      s2: "Mod catalog",
      s2alt: "Mod catalog illustration",
      s3: "Rooms, chat and voice",
      s3alt: "Room with chat illustration",
      dlTitle: "Download",
      dlLatest: "Latest release",
      dlVersion: "Latest release: {v}",
      dlFallback: "Could not reach GitHub — the buttons open the releases page.",
      dlExe: "Installer",
      dlAll: "All releases and changelog",
      issues: "Report a bug",
      disclaimer: "Not an official Minecraft product. Not affiliated with Mojang or Microsoft."
    },
    ru: {
      title: "MineUP — лаунчер Minecraft",
      skip: "К содержимому",
      navLabel: "Основное",
      navFeatures: "Возможности",
      navScreens: "Интерфейс",
      navDownload: "Скачать",
      sceneBtn: "Сменить фон",
      langBtn: "English",
      logoAlt: "Логотип MineUP",
      pitch: "Быстрый бесплатный лаунчер Minecraft: моды, скины, игра с друзьями по коду комнаты и встроенный VPN.",
      ctaDownload: "Скачать",
      ctaGithub: "Исходники на GitHub",
      platforms: "Windows 10/11 и Linux · бесплатно и с открытым кодом (GPL-3.0)",
      featuresTitle: "Всё в одном лаунчере",
      f1t: "Версии и загрузчики",
      f1d: "Любая версия Minecraft, Fabric, Quilt, Forge и NeoForge в один клик. Java ставится автоматически.",
      f2t: "Моды и модпаки",
      f2d: "Каталог Modrinth прямо в приложении: моды, модпаки, ресурспаки и шейдеры с разрешением зависимостей.",
      f3t: "Свой сервер скинов",
      f3d: "Загружайте скины и плащи, смотрите их в 3D — и их видят все на сервере. Аккаунт Microsoft не нужен.",
      f4t: "P2P-мультиплеер",
      f4d: "Создайте комнату, поделитесь кодом и играйте вместе. Прямое соединение с ретранслятором про запас, без проброса портов.",
      f5t: "Голос и чат",
      f5d: "Встроенный чат и голос в комнате, push-to-talk и оверлей поверх игры.",
      f6t: "VPN и обход блокировок",
      f6d: "Загрузки и сервисы работают даже там, где их блокируют: встроенный VPN, зеркала и умная маршрутизация.",
      f7t: "Режим для слабых ПК",
      f7d: "Лёгкий режим без тяжёлых эффектов и подобранные настройки памяти и JVM для слабого железа.",
      f8t: "Автообновления",
      f8d: "Лаунчер сам обновляется в фоне, релизы подписаны.",
      screensTitle: "Как это выглядит",
      screensNote: "Иллюстрации интерфейса, нарисованные на CSS, а не настоящие скриншоты.",
      s1: "Библиотека сборок",
      s1alt: "Иллюстрация библиотеки сборок",
      s2: "Каталог модов",
      s2alt: "Иллюстрация каталога модов",
      s3: "Комнаты, чат и голос",
      s3alt: "Иллюстрация комнаты с чатом",
      dlTitle: "Скачать",
      dlLatest: "Последний релиз",
      dlVersion: "Последний релиз: {v}",
      dlFallback: "GitHub недоступен — кнопки откроют страницу релизов.",
      dlExe: "Установщик",
      dlAll: "Все релизы и список изменений",
      issues: "Сообщить об ошибке",
      disclaimer: "Не является официальным продуктом Minecraft. Не связан с Mojang или Microsoft."
    }
  };

  function store(key, value) {
    try {
      if (value === undefined) return window.localStorage.getItem(key);
      window.localStorage.setItem(key, value);
    } catch (e) {
      /* storage disabled */
    }
    return null;
  }

  var saved = store("mineup.lang");
  var lang = saved === "ru" || saved === "en" ? saved
    : /^ru\b|^be\b|^uk\b|^kk\b/i.test(navigator.language || "") ? "ru" : "en";
  var release = null;
  var releaseFailed = false;

  function t(key) {
    return (DICT[lang] && DICT[lang][key]) || DICT.en[key] || key;
  }

  function applyLang() {
    document.documentElement.lang = lang;
    document.title = t("title");
    document.querySelectorAll("[data-i18n]").forEach(function (el) {
      el.textContent = t(el.getAttribute("data-i18n"));
    });
    document.querySelectorAll("[data-i18n-aria]").forEach(function (el) {
      el.setAttribute("aria-label", t(el.getAttribute("data-i18n-aria")));
    });
    document.querySelectorAll("[data-i18n-alt]").forEach(function (el) {
      el.setAttribute("alt", t(el.getAttribute("data-i18n-alt")));
    });
    var btn = document.getElementById("lang-btn");
    btn.textContent = lang === "ru" ? "EN" : "RU";
    btn.setAttribute("aria-label", t("langBtn"));
    btn.setAttribute("lang", lang === "ru" ? "en" : "ru");
    var v = document.getElementById("dl-version");
    if (release) v.textContent = t("dlVersion").replace("{v}", release.tag_name || "");
    else if (releaseFailed) v.textContent = t("dlFallback");
  }

  document.getElementById("lang-btn").addEventListener("click", function () {
    lang = lang === "ru" ? "en" : "ru";
    store("mineup.lang", lang);
    applyLang();
  });

  // Backgrounds: page scene follows the colour scheme unless the user picked one.
  var S = window.MineupScenes;
  var reduce = window.matchMedia("(prefers-reduced-motion: reduce)");
  var dark = window.matchMedia("(prefers-color-scheme: dark)");
  var bg = document.getElementById("bg");
  var scene = store("mineup.scene");

  function paintBg() {
    var id = scene && S.ids.indexOf(scene) >= 0 ? scene : dark.matches ? "forest" : "plains";
    bg.innerHTML = S.render(id, !reduce.matches);
    bg.setAttribute("data-scene", id);
  }

  function paintMocks() {
    document.querySelectorAll(".mock[data-scene]").forEach(function (el) {
      var old = el.querySelector(".bg");
      if (old) old.remove();
      var d = document.createElement("div");
      d.className = "bg";
      d.setAttribute("aria-hidden", "true");
      d.innerHTML = S.render(el.getAttribute("data-scene"), false);
      el.insertBefore(d, el.firstChild);
    });
  }

  document.getElementById("scene-btn").addEventListener("click", function () {
    var cur = bg.getAttribute("data-scene");
    scene = S.ids[(S.ids.indexOf(cur) + 1) % S.ids.length];
    store("mineup.scene", scene);
    paintBg();
  });

  function onChange(mq, fn) {
    if (mq.addEventListener) mq.addEventListener("change", fn);
    else if (mq.addListener) mq.addListener(fn);
  }
  onChange(dark, paintBg);
  onChange(reduce, paintBg);

  // Latest release assets; buttons already point at /releases/latest as a fallback.
  var PATTERNS = {
    exe: /\.exe$/i,
    msi: /\.msi$/i,
    appimage: /\.AppImage$/i,
    deb: /\.deb$/i
  };

  function loadRelease() {
    if (!window.fetch) return;
    fetch("https://api.github.com/repos/" + REPO + "/releases/latest", {
      headers: { Accept: "application/vnd.github+json" }
    })
      .then(function (r) {
        if (!r.ok) throw new Error("HTTP " + r.status);
        return r.json();
      })
      .then(function (rel) {
        var assets = (rel && rel.assets) || [];
        document.querySelectorAll("[data-asset]").forEach(function (a) {
          var re = PATTERNS[a.getAttribute("data-asset")];
          var hit = assets.filter(function (x) {
            return re.test(x.name) && !/\.sig$/i.test(x.name);
          })[0];
          if (hit && /^https:\/\/github\.com\//.test(hit.browser_download_url)) {
            a.href = hit.browser_download_url;
            a.title = hit.name;
          }
        });
        release = rel;
        applyLang();
      })
      .catch(function () {
        releaseFailed = true;
        applyLang();
      });
  }

  applyLang();
  paintBg();
  paintMocks();
  loadRelease();
})();
