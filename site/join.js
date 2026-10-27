// https://mineded.dedseckms.ru/join/<code> and /invite/<code> → mineded:// deep link.
// Telegram and most messengers don't make custom schemes clickable, so the
// launcher shares these https links instead.
(function () {
  try { document.getElementById("bg").innerHTML = window.MineupScenes.render("forest", false); } catch (e) {}
  var m = location.pathname.match(/^\/(join|invite)\/([^/?#]+)/);
  if (!m) { location.replace("/"); return; }
  var kind = m[1], code = decodeURIComponent(m[2]);
  if (!/^[A-Za-z0-9-]{4,64}$/.test(code)) { location.replace("/"); return; }
  var link = "mineded://" + kind + "/" + encodeURIComponent(code);
  document.getElementById("code").textContent = kind === "join" ? code.toUpperCase() : code;
  document.getElementById("open").href = link;
  if (kind === "invite") {
    document.getElementById("title").textContent = "Инвайт на сервер MineDed";
  }
  // Try to open the launcher; if the page is still visible afterwards, it isn't installed.
  location.href = link;
  setTimeout(function () {
    if (!document.hidden) {
      document.getElementById("hint").textContent = "Если лаунчер установлен, нажмите «Открыть в лаунчере».";
    }
  }, 1500);
})();
