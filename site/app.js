// SPDX-License-Identifier: GPL-3.0-or-later
(function () {
  "use strict";
  var REPO = "Dedseckms/MineDed";

  var DICT = {
    en: {
      title: "MineDed — free Minecraft launcher with mods, skins and multiplayer with friends",
      h1: "MineDed — Minecraft launcher",
      skip: "Skip to content",
      navLabel: "Main",
      navFeatures: "Features",
      navScreens: "Screens",
      navFaq: "FAQ",
      navDownload: "Download",
      sceneBtn: "Change background",
      langBtn: "Русский",
      logoAlt: "MineDed logo",
      pitch: "A fast, free Minecraft launcher with mods, skins, voice chat and multiplayer by room code.",
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
      f4d: "Create a room, share the code — your friend downloads your modpack automatically and joins. Direct connection with relay fallback, no port forwarding.",
      f5t: "Voice & chat",
      f5d: "Built-in room chat and voice with noise suppression, push-to-talk and an in-game overlay.",
      f6t: "Manage your instances",
      f6d: "Create and clone instances, then export and import .mrpack files to move them between computers.",
      f7t: "Low-end PC mode",
      f7d: "A light mode without heavy effects, plus tuned memory and JVM flags for weak hardware.",
      f8t: "Auto-updates",
      f8d: "The launcher updates itself in the background with signed releases.",
      screensTitle: "How it looks",
      screensNote: "Real screenshots of the launcher. Click to enlarge.",
      shPlay: "Play",
      shPlayAlt: "Play screen: instance and account picker, running game with a live log",
      shInstances: "Instance library",
      shInstancesAlt: "Instance library with vanilla, Fabric and Forge instances",
      shModpacks: "Modpack catalog",
      shModpacksAlt: "Modpack catalog from Modrinth with filters",
      shMods: "Instance mods",
      shModsAlt: "Installed mods of a Fabric instance with updates and toggles",
      shCatalog: "Mod catalog",
      shCatalogAlt: "Modrinth mod catalog filtered by the instance version and loader",
      shSkins: "Skins in 3D",
      shSkinsAlt: "Skin library with a 3D preview and a cape",
      shAccounts: "Accounts",
      shAccountsAlt: "Offline accounts and sign-in to the MineDed server",
      shMultiplayer: "Room, voice and chat",
      shMultiplayerAlt: "Multiplayer room with a code, participants, voice panel and chat",
      shSettings: "Appearance",
      shSettingsAlt: "Appearance settings: theme, accent colour, language and pixel backgrounds",
      lbPrev: "Previous screenshot",
      lbNext: "Next screenshot",
      lbClose: "Close",
      faqTitle: "FAQ",
      q1: "How do I play Minecraft with a friend without a server?",
      a1: "Create a room in the Multiplayer section and send your friend its code. MineDed connects you directly (P2P), falling back to a relay when needed. No server rental or port forwarding. Text and voice chat are built in.",
      q2: "Do I need a Minecraft licence or a Microsoft account?",
      a2: "No. Play with an offline account or sign in to the MineDed server so other players see your skin and cape. A Microsoft account is not required.",
      q3: "Will MineDed run on a low-end PC?",
      a3: "Yes. The launcher itself is light, and the low-end PC mode turns off heavy effects. Memory and JVM flags are tuned automatically, and Java installs itself.",
      q4: "Does the launcher work on Linux?",
      a4: "Yes. MineDed runs on Windows 10/11 and Linux: there are .exe and .msi installers, an AppImage and a .deb package for Debian and Ubuntu.",
      q5: "How do I move an instance to another computer?",
      a5: "Export the instance as a .mrpack file, copy it to the other computer and import it in MineDed.",
      dlTitle: "Download",
      dlLatest: "Latest release",
      dlVersion: "Latest release: {v}",
      dlFallback: "Could not reach GitHub — the buttons open the releases page.",
      dlExe: "Installer",
      dlAll: "All releases and changelog",
      dlSkins: "Skin server account: sign in, skin and cape",
      signIn: "Sign in",
      issues: "Report a bug",
      privacy: "Privacy policy",
      disclaimer: "Not an official Minecraft product. Not affiliated with Mojang or Microsoft."
    },
    ru: {
      title: "MineDed — бесплатный лаунчер Minecraft с модами, скинами и игрой с друзьями",
      h1: "MineDed — лаунчер Minecraft",
      skip: "К содержимому",
      navLabel: "Основное",
      navFeatures: "Возможности",
      navScreens: "Интерфейс",
      navFaq: "Вопросы",
      navDownload: "Скачать",
      sceneBtn: "Сменить фон",
      langBtn: "English",
      logoAlt: "Логотип MineDed",
      pitch: "Быстрый бесплатный лаунчер Minecraft: моды, скины, голосовой чат и игра с друзьями по коду комнаты.",
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
      f4d: "Создайте комнату и отправьте код — друг сам скачает вашу сборку и подключится. Прямое соединение с ретранслятором про запас, без проброса портов.",
      f5t: "Голос и чат",
      f5d: "Встроенный чат и голос с шумоподавлением, push-to-talk и оверлей поверх игры.",
      f6t: "Управление сборками",
      f6d: "Создавайте и копируйте сборки, переносите их между компьютерами через экспорт и импорт .mrpack.",
      f7t: "Режим для слабых ПК",
      f7d: "Лёгкий режим без тяжёлых эффектов и подобранные настройки памяти и JVM для слабого железа.",
      f8t: "Автообновления",
      f8d: "Лаунчер сам обновляется в фоне, релизы подписаны.",
      screensTitle: "Как это выглядит",
      screensNote: "Настоящие скриншоты лаунчера. Нажмите, чтобы увеличить.",
      shPlay: "Запуск игры",
      shPlayAlt: "Экран «Играть»: выбор сборки и аккаунта, запущенная игра и её лог",
      shInstances: "Библиотека сборок",
      shInstancesAlt: "Библиотека сборок: ванилла, Fabric и Forge",
      shModpacks: "Каталог модпаков",
      shModpacksAlt: "Каталог модпаков Modrinth с фильтрами",
      shMods: "Моды сборки",
      shModsAlt: "Установленные моды сборки Fabric с обновлениями и переключателями",
      shCatalog: "Каталог модов",
      shCatalogAlt: "Каталог модов Modrinth с фильтром по версии и загрузчику сборки",
      shSkins: "Скины в 3D",
      shSkinsAlt: "Библиотека скинов с 3D-просмотром и плащом",
      shAccounts: "Аккаунты",
      shAccountsAlt: "Оффлайн-аккаунты и вход через сервер MineDed",
      shMultiplayer: "Комната, голос и чат",
      shMultiplayerAlt: "Комната мультиплеера: код, участники, голосовая панель и чат",
      shSettings: "Внешний вид",
      shSettingsAlt: "Настройки внешнего вида: тема, акцент, язык и пиксельные фоны",
      lbPrev: "Предыдущий скриншот",
      lbNext: "Следующий скриншот",
      lbClose: "Закрыть",
      faqTitle: "Частые вопросы",
      q1: "Как играть в Minecraft с другом без сервера?",
      a1: "Создайте комнату в разделе «Мультиплеер» и отправьте другу её код. MineDed соединит вас напрямую (P2P), а при необходимости — через ретранслятор. Аренда сервера и проброс портов не нужны. Чат и голосовой чат встроены.",
      q2: "Нужна ли лицензия Minecraft или аккаунт Microsoft?",
      a2: "Нет. Можно играть с оффлайн-аккаунтом или войти через сервер MineDed — тогда ваш скин и плащ увидят и другие игроки. Аккаунт Microsoft не требуется.",
      q3: "Пойдёт ли MineDed на слабом ПК?",
      a3: "Да. Сам лаунчер лёгкий, а режим для слабых ПК отключает тяжёлые эффекты. Память и параметры JVM подбираются автоматически, Java ставится сама.",
      q4: "Работает ли лаунчер на Linux?",
      a4: "Да. MineDed работает на Windows 10/11 и Linux: доступны установщик .exe и .msi, а также AppImage и пакет .deb для Debian и Ubuntu.",
      q5: "Как перенести сборку на другой компьютер?",
      a5: "Экспортируйте сборку в файл .mrpack, перенесите его на другой компьютер и импортируйте в MineDed.",
      dlTitle: "Скачать",
      dlLatest: "Последний релиз",
      dlVersion: "Последний релиз: {v}",
      dlFallback: "GitHub недоступен — кнопки откроют страницу релизов.",
      dlExe: "Установщик",
      dlAll: "Все релизы и список изменений",
      dlSkins: "Аккаунт на сервере скинов: вход, скин и плащ",
      signIn: "Войти",
      issues: "Сообщить об ошибке",
      privacy: "Политика конфиденциальности",
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
  // Static HTML is Russian. Switch to English only for non-RU-family visitors;
  // crawlers keep the Russian page whatever their navigator.language says.
  var bot = /bot|crawl|spider|slurp|yandex|google|bing|lighthouse/i.test(navigator.userAgent || "");
  var lang = saved === "ru" || saved === "en" ? saved
    : bot || /^ru\b|^be\b|^uk\b|^kk\b/i.test(navigator.language || "") ? "ru" : "en";
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
    paintShots();
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

  // Screenshot gallery: per-language images, native <dialog> lightbox.
  var shots = Array.prototype.slice.call(document.querySelectorAll("[data-shot]"));
  var lightbox = document.getElementById("lightbox");
  var lbImg = document.getElementById("lb-img");
  var lbCaption = document.getElementById("lb-caption");
  var lbIndex = 0;

  function shotKey(id) {
    return "sh" + id.charAt(0).toUpperCase() + id.slice(1);
  }

  function shotSrc(id) {
    return "screenshots/" + id + "-" + lang + ".webp?v=6";
  }

  function paintShots() {
    shots.forEach(function (btn) {
      var img = btn.querySelector("img");
      var src = shotSrc(btn.getAttribute("data-shot"));
      if (img.getAttribute("src") !== src) img.setAttribute("src", src);
    });
    if (lightbox.open) showShot(lbIndex);
  }

  function showShot(i) {
    lbIndex = (i + shots.length) % shots.length;
    var id = shots[lbIndex].getAttribute("data-shot");
    lbImg.src = shotSrc(id);
    lbImg.alt = t(shotKey(id) + "Alt");
    lbCaption.textContent = t(shotKey(id));
  }

  shots.forEach(function (btn, i) {
    btn.addEventListener("click", function () {
      showShot(i);
      if (typeof lightbox.showModal === "function") lightbox.showModal();
      else window.open(shotSrc(btn.getAttribute("data-shot")), "_blank", "noopener");
    });
  });
  lightbox.addEventListener("click", function (e) {
    var action = e.target.closest ? e.target.closest("[data-lb]") : null;
    if (e.target === lightbox) lightbox.close(); // click on the backdrop
    else if (action) {
      var a = action.getAttribute("data-lb");
      if (a === "close") lightbox.close();
      else showShot(lbIndex + (a === "next" ? 1 : -1));
    }
  });
  lightbox.addEventListener("keydown", function (e) {
    if (e.key === "ArrowRight") showShot(lbIndex + 1);
    else if (e.key === "ArrowLeft") showShot(lbIndex - 1);
  });

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
  loadRelease();
})();
