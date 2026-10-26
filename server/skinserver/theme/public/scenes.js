// SPDX-License-Identifier: GPL-3.0-or-later
// Voxel backgrounds, ported 1:1 from apps/desktop/src/features/appearance/backgrounds.tsx
// (same scene data and generator), rendered to SVG strings without a framework.
(function () {
  "use strict";
  var W = 320;
  var H = 180;
  var VIEW = "0 0 " + W + " " + H;
  var uidSeq = 0;

  function heights(h) {
    var out = [];
    for (var x = 0; x <= W; x += h.step) {
      var n = 0.6 * Math.sin(x * 0.021 + h.seed) + 0.4 * Math.sin(x * 0.057 + h.seed * 1.7);
      out.push(Math.round((h.base + h.amp * n) / h.step) * h.step);
    }
    return out;
  }

  function stepPath(hs, step, ceiling, offset) {
    offset = offset || 0;
    var y = function (v) {
      return ceiling ? v - offset : H - v + offset;
    };
    var d = "M0 " + (ceiling ? 0 : H);
    hs.forEach(function (v, i) {
      d += "V" + y(v) + "H" + Math.min(W, (i + 1) * step);
    });
    return d + "V" + (ceiling ? 0 : H) + "Z";
  }

  function hill(h, hs) {
    var c = !!h.ceiling;
    var capH = h.capH || h.step;
    if (h.cap) {
      return (
        '<path d="' + stepPath(hs, h.step, c) + '" fill="' + h.cap + '"/>' +
        '<path d="' + stepPath(hs, h.step, c, capH) + '" fill="' + h.color + '"/>'
      );
    }
    return '<path d="' + stepPath(hs, h.step, c) + '" fill="' + h.color + '"/>';
  }

  function rng(seed) {
    var a = seed >>> 0;
    return function () {
      a = (a + 0x6d2b79f5) >>> 0;
      var t = a;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  function rect(x, y, w, h, fill, extra) {
    return '<rect x="' + x + '" y="' + y + '" width="' + w + '" height="' + h + '"' +
      (fill ? ' fill="' + fill + '"' : "") + (extra || "") + "/>";
  }

  function topAt(hs, step, x) {
    return H - (hs[Math.floor(x / step)] || 0);
  }

  function trees(hs, step, trunk, leaves, every, seed) {
    var r = rng(seed);
    var s = "";
    for (var x = 10; x < W - 10; x += every + Math.floor(r() * every)) {
      var gx = Math.floor(x / 4) * 4;
      var y = topAt(hs, step, gx);
      var tall = 12 + Math.floor(r() * 3) * 4;
      s += rect(gx, y - tall, 4, tall, trunk) +
        rect(gx - 8, y - tall - 8, 20, 12, leaves[0]) +
        rect(gx - 4, y - tall - 14, 12, 6, leaves[1] || leaves[0]);
    }
    return s;
  }

  function clouds(color) {
    var shapes = [[40, 26, 36], [120, 16, 48], [196, 34, 28], [252, 20, 40]];
    return '<g fill="' + color + '">' + shapes.map(function (c) {
      return rect(c[0], c[1], c[2], 8) + rect(c[0] + 8, c[1] - 4, c[2] - 16, 4);
    }).join("") + "</g>";
  }

  function stars(color, seed) {
    var r = rng(seed);
    var s = '<g fill="' + color + '">';
    for (var i = 0; i < 36; i++) {
      var sz = i % 5 ? 1 : 2;
      var x = Math.floor(r() * 160) * 2;
      var y = Math.floor(r() * 40) * 2;
      s += rect(x, y, sz, sz, null, ' opacity="' + (0.5 + r() * 0.5).toFixed(2) + '"');
    }
    return s + "</g>";
  }

  var SCENES = {
    plains: {
      sky: ["#f9a66c", "#f7c59f", "#9fd3ef"],
      orb: { x: 236, y: 52, s: 20, color: "#ffe9a8", glow: "#ffd27a" },
      clouds: "#fff4e6",
      far: [{ color: "#6f9f8f", base: 70, amp: 14, step: 8, seed: 1 }],
      near: [{ color: "#7a5532", cap: "#5fae3c", capH: 6, base: 44, amp: 10, step: 8, seed: 4 }]
    },
    forest: {
      sky: ["#0b1330", "#1a2a55", "#2c4068"],
      orb: { x: 288, y: 36, s: 16, color: "#e8ecf5", glow: "#9fb4e0" },
      stars: "#dfe6ff",
      far: [{ color: "#1d3346", base: 74, amp: 12, step: 8, seed: 2 }],
      near: [{ color: "#2a1d14", cap: "#1f4a2a", capH: 4, base: 40, amp: 8, step: 8, seed: 7 }],
      extra: function (n) { return trees(n[0] || [], 8, "#3b2a1c", ["#173a22", "#1f4d2c"], 22, 11); }
    },
    nether: {
      sky: ["#2a0606", "#5a0f0c", "#a3260f"],
      far: [
        { color: "#3d0a0a", base: 46, amp: 12, step: 8, seed: 3, ceiling: true },
        { color: "#5b1410", base: 64, amp: 16, step: 8, seed: 5 }
      ],
      near: [
        { color: "#6e1712", cap: "#8a1f17", capH: 4, base: 34, amp: 18, step: 8, seed: 9 },
        { color: "#ff7a1a", cap: "#ffb347", capH: 2, base: 22, amp: 0, step: 8, seed: 0 }
      ],
      extra: function () {
        var r = rng(5);
        var s = '<g fill="#ffd36b">';
        for (var i = 0; i < 7; i++) {
          s += rect(Math.floor(r() * 76) * 4, 4 + Math.floor(r() * 6) * 4, 4, 4, null, ' opacity="0.9"');
        }
        return s + "</g>";
      }
    },
    ocean: {
      sky: ["#5fb7f2", "#a7dcf7", "#d8f1fb"],
      orb: { x: 284, y: 36, s: 18, color: "#fff6c7", glow: "#ffe58a" },
      clouds: "#ffffff",
      far: [{ color: "#7fb08a", cap: "#e9d9a1", capH: 3, base: 66, amp: 6, step: 8, seed: 6 }],
      near: [
        { color: "#2d7fc1", base: 62, amp: 0, step: 8, seed: 0 },
        { color: "#2369a6", base: 40, amp: 0, step: 8, seed: 0 },
        { color: "#1c548a", base: 18, amp: 0, step: 8, seed: 0 }
      ],
      extra: function () {
        return '<g fill="#bfe6ff" opacity="0.7">' + [30, 90, 150, 210, 270].map(function (x, i) {
          return rect(x, 124 + (i % 2) * 22, 16, 2);
        }).join("") + "</g>";
      }
    },
    snowy: {
      sky: ["#7d9cc4", "#b8cbe2", "#e4edf6"],
      clouds: "#f4f8fc",
      far: [{ color: "#8697ad", cap: "#f2f6fb", capH: 12, base: 104, amp: 34, step: 8, seed: 8 }],
      near: [{ color: "#5f6c7c", cap: "#ffffff", capH: 8, base: 50, amp: 16, step: 8, seed: 12 }],
      extra: function (n) { return trees(n[0] || [], 8, "#3f3226", ["#2f5546", "#e9f1f7"], 34, 3); }
    },
    cave: {
      sky: ["#16181c", "#22252b", "#2b2f36"],
      far: [
        { color: "#33373e", base: 40, amp: 12, step: 8, seed: 4, ceiling: true },
        { color: "#3a3f47", base: 52, amp: 10, step: 8, seed: 10 }
      ],
      near: [
        { color: "#4b5059", base: 30, amp: 14, step: 8, seed: 13, ceiling: true },
        { color: "#4f545d", cap: "#5d636d", capH: 4, base: 34, amp: 12, step: 8, seed: 2 }
      ],
      extra: function (n, uid) {
        var r = rng(21);
        var ores = ["#5ee0e6", "#f2c94c", "#e24b4b", "#4cd07d"];
        var t = "torch-" + uid;
        var s = '<radialGradient id="' + t + '" cx="0.5" cy="0.5" r="0.5">' +
          '<stop offset="0" stop-color="#ffb347" stop-opacity="0.35"/>' +
          '<stop offset="1" stop-color="#ffb347" stop-opacity="0"/></radialGradient>' +
          rect(120, 60, 80, 80, "url(#" + t + ")") + rect(158, 96, 2, 8, "#7a5532") + rect(157, 92, 4, 4, "#ffd36b");
        for (var i = 0; i < 10; i++) {
          var x = Math.floor(r() * 76) * 4;
          var y = i % 2 ? 150 + Math.floor(r() * 6) * 4 : 6 + Math.floor(r() * 4) * 4;
          s += '<g fill="' + ores[i % 4] + '">' + rect(x, y, 4, 4) + rect(x + 6, y + 2, 2, 2) + rect(x + 2, y + 6, 4, 2) + "</g>";
        }
        return s;
      }
    },
    desert: {
      sky: ["#3b2a5c", "#c0607a", "#f5a25d"],
      orb: { x: 160, y: 104, s: 26, color: "#ffd27a", glow: "#ff9f5a" },
      stars: "#f6e7ff",
      far: [{ color: "#c98a52", base: 60, amp: 8, step: 8, seed: 14 }],
      near: [{ color: "#d9a35f", cap: "#ecc27c", capH: 4, base: 36, amp: 8, step: 8, seed: 15 }],
      extra: function (n) {
        var hs = n[0] || [];
        return '<g fill="#3f7a3a">' + [48, 212, 276].map(function (x) {
          var y = topAt(hs, 8, x);
          return rect(x, y - 20, 4, 20) + rect(x - 4, y - 14, 4, 6) + rect(x + 4, y - 16, 4, 6);
        }).join("") + "</g>";
      }
    },
    cherry: {
      sky: ["#9ec9f0", "#d8e6f6", "#fbe3ee"],
      orb: { x: 252, y: 40, s: 16, color: "#fff7d6", glow: "#ffe9b0" },
      clouds: "#ffffff",
      far: [{ color: "#9cb98d", base: 66, amp: 12, step: 8, seed: 16 }],
      near: [{ color: "#6b4a33", cap: "#79b85a", capH: 5, base: 42, amp: 9, step: 8, seed: 17 }],
      extra: function (n) { return trees(n[0] || [], 8, "#5a3b2e", ["#f4a6c6", "#f9c8dc"], 30, 8); }
    }
  };

  function svg(body, extraAttr) {
    return '<svg xmlns="http://www.w3.org/2000/svg" viewBox="' + VIEW +
      '" preserveAspectRatio="xMidYMax slice" shape-rendering="crispEdges" class="layer"' +
      (extraAttr || "") + ">" + body + "</svg>";
  }

  /** Returns the HTML of a scene (sky, drifting clouds, swaying far hills, foreground). */
  function render(id, animate) {
    var sc = SCENES[id] || SCENES.plains;
    var uid = "s" + uidSeq++;
    var far = sc.far.map(heights);
    var near = sc.near.map(heights);
    var defs = '<defs><linearGradient id="sky-' + uid + '" x1="0" y1="0" x2="0" y2="1">' +
      sc.sky.map(function (c, i) {
        return '<stop offset="' + i / Math.max(1, sc.sky.length - 1) + '" stop-color="' + c + '"/>';
      }).join("") + "</linearGradient>" +
      (sc.orb ? '<radialGradient id="glow-' + uid + '" cx="0.5" cy="0.5" r="0.5"><stop offset="0" stop-color="' +
        sc.orb.glow + '" stop-opacity="0.55"/><stop offset="1" stop-color="' + sc.orb.glow +
        '" stop-opacity="0"/></radialGradient>' : "") + "</defs>";
    var back = defs + rect(0, 0, W, H, "url(#sky-" + uid + ")") +
      (sc.stars ? stars(sc.stars, id.length * 7) : "");
    if (sc.orb) {
      var o = sc.orb;
      back += rect(o.x - o.s * 2, o.y - o.s * 2, o.s * 4, o.s * 4, "url(#glow-" + uid + ")") +
        rect(o.x - o.s / 2, o.y - o.s / 4, o.s, o.s / 2, o.color) +
        rect(o.x - o.s * 3 / 8, o.y - o.s * 3 / 8, o.s * 3 / 4, o.s * 3 / 4, o.color) +
        rect(o.x - o.s / 4, o.y - o.s / 2, o.s / 2, o.s, o.color);
    }
    var html = svg(back);
    if (sc.clouds) {
      var c = svg(clouds(sc.clouds), ' opacity="0.85"');
      html += '<div class="clouds' + (animate ? " bg-drift" : "") + '"><div>' + c + "</div><div>" + c + "</div></div>";
    }
    html += '<div class="far' + (animate ? " bg-sway" : "") + '">' +
      svg(sc.far.map(function (h, i) { return hill(h, far[i]); }).join("")) + "</div>";
    html += svg(sc.near.map(function (h, i) { return hill(h, near[i]); }).join("") +
      (sc.extra ? sc.extra(near, uid) : ""));
    return html;
  }

  window.MineupScenes = { render: render, ids: Object.keys(SCENES) };
})();
