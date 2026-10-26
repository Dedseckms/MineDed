// SPDX-License-Identifier: GPL-3.0-or-later
// MineDed theme for Drasl: voxel background scene (scenes.js) that follows the
// colour scheme, with a header button to cycle scenes; launcher deep links and
// the registration password confirmation (below).

// Launcher links and forms. Pages work without JS: the confirmation field has
// no name (never sent) and deep links are plain anchors.
(function () {
  "use strict";
  var FALLBACK_MS = 1500;

  // `mineded://…` anchors with `data-fallback`: if the page is still visible
  // and focused after FALLBACK_MS (no launcher took the link), go to download.
  function bindLauncherLink(a) {
    a.addEventListener("click", function () {
      var fallback = a.getAttribute("data-fallback");
      if (!fallback) return;
      var timer = window.setTimeout(function () {
        if (document.visibilityState === "visible" && document.hasFocus()) window.location.href = fallback;
      }, FALLBACK_MS);
      var cancel = function () { window.clearTimeout(timer); };
      window.addEventListener("blur", cancel, { once: true });
      window.addEventListener("pagehide", cancel, { once: true });
      document.addEventListener("visibilitychange", cancel, { once: true });
    });
  }
  Array.prototype.forEach.call(document.querySelectorAll("a[data-launcher-link]"), bindLauncherLink);

  // Invite links anywhere (admin invite list): add "open in launcher" next to them.
  var ru = (document.documentElement.lang || "").indexOf("ru") === 0;
  Array.prototype.forEach.call(document.querySelectorAll('a[href*="/web/registration?invite="]'), function (a) {
    var code;
    try { code = new URL(a.href, window.location.href).searchParams.get("invite"); } catch (e) { code = null; }
    if (!code || !/^[A-Za-z0-9_-]+$/.test(code)) return;
    var link = document.createElement("a");
    link.className = "launcher-inline";
    link.href = "mineded://invite/" + code;
    link.setAttribute("data-fallback", "https://mineded.dedseckms.ru/#download");
    link.textContent = ru ? "Открыть в лаунчере" : "Open in launcher";
    a.insertAdjacentElement("afterend", link);
    bindLauncherLink(link);
  });

  // Registration: the repeat field must match "password"; native validation
  // then blocks the submit and shows the message.
  Array.prototype.forEach.call(document.querySelectorAll("form[data-confirm-password]"), function (form) {
    var pw = form.querySelector('input[name="password"]');
    var confirm = form.querySelector("input[data-password-confirm]");
    if (!pw || !confirm) return;
    var msg = confirm.getAttribute("data-mismatch") || "Passwords do not match";
    var check = function () { confirm.setCustomValidity(confirm.value && confirm.value !== pw.value ? msg : ""); };
    pw.addEventListener("input", check);
    confirm.addEventListener("input", check);
    form.addEventListener("submit", function (e) {
      check();
      if (confirm.value !== pw.value) {
        e.preventDefault();
        confirm.setCustomValidity(msg);
        confirm.reportValidity();
      }
    });
  });
})();

(function () {
  "use strict";
  var S = window.MineupScenes;
  var bg = document.getElementById("background");
  if (!S || !bg) return;
  var reduce = window.matchMedia("(prefers-reduced-motion: reduce)");
  var dark = window.matchMedia("(prefers-color-scheme: dark)");
  var KEY = "mineded.scene";

  function load() {
    try { return window.localStorage.getItem(KEY); } catch (e) { return null; }
  }
  function save(v) {
    try { window.localStorage.setItem(KEY, v); } catch (e) { /* storage disabled */ }
  }

  var scene = load();

  function paint() {
    var id = scene && S.ids.indexOf(scene) >= 0 ? scene : dark.matches ? "forest" : "plains";
    bg.innerHTML = S.render(id, !reduce.matches);
    bg.setAttribute("data-scene", id);
  }

  var btn = document.getElementById("scene-btn");
  if (btn) {
    btn.hidden = false;
    btn.addEventListener("click", function () {
      var cur = bg.getAttribute("data-scene");
      scene = S.ids[(S.ids.indexOf(cur) + 1) % S.ids.length];
      save(scene);
      paint();
    });
  }

  function onChange(mq, fn) {
    if (mq.addEventListener) mq.addEventListener("change", fn);
    else if (mq.addListener) mq.addListener(fn);
  }
  onChange(dark, paint);
  onChange(reduce, paint);
  paint();
})();
