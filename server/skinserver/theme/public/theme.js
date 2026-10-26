// SPDX-License-Identifier: GPL-3.0-or-later
// MineDed theme for Drasl: voxel background scene (scenes.js) that follows the
// colour scheme, with a header button to cycle scenes. No other behaviour.
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
