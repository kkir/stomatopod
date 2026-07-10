(function () {
  "use strict";

  // Avoid double-init when the script is included more than once.
  if (window.stomatopod && window.stomatopod.l) return;

  if (navigator.doNotTrack === "1" || window.doNotTrack === "1") return;

  var script =
    document.currentScript || document.querySelector("script[data-site]");
  if (!script || script.hasAttribute("data-exclude")) return;

  var siteKey = script.getAttribute("data-site");
  if (!siteKey) return;

  // Prefer data-api; otherwise post to the same origin the script was loaded
  // from (so a cross-origin <script src="https://analytics.example/tracker.js">
  // does not hit the page's own /api/v1/event).
  var endpoint = script.getAttribute("data-api");
  if (!endpoint) {
    try {
      endpoint = script.src
        ? new URL(script.src).origin + "/api/v1/event"
        : "/api/v1/event";
    } catch (_) {
      endpoint = "/api/v1/event";
    }
  }

  // Last path we already counted as a pageview. Used to ignore replaceState
  // noise (scroll-spy hash updates, UI query tweaks) and duplicate SPA navs.
  var lastPath = null;

  function pathKey() {
    // Hash-only changes are not pageviews for path-based sites.
    return location.pathname + location.search;
  }

  function send(name, props) {
    var payload = JSON.stringify({
      k: siteKey,
      n: name,
      u: location.href,
      r: document.referrer || undefined,
      w: screen.width,
      h: screen.height,
      l: navigator.language || undefined,
      p: props || undefined,
      t: Date.now(),
    });
    var blob = new Blob([payload], { type: "application/json" });
    // sendBeacon can return false when the browser refuses the request
    // (e.g. quota); fall back to keepalive fetch.
    if (navigator.sendBeacon && navigator.sendBeacon(endpoint, blob)) {
      return;
    }
    try {
      fetch(endpoint, {
        method: "POST",
        body: payload,
        keepalive: true,
        headers: { "Content-Type": "application/json" },
        credentials: "omit",
      });
    } catch (_) {
      /* best-effort */
    }
  }

  function pageview() {
    var key = pathKey();
    if (key === lastPath) return;
    lastPath = key;
    send("pageview");
  }

  function onNav() {
    // Defer so location.href reflects the new history entry.
    setTimeout(pageview, 0);
  }

  // Initial pageview. During prerender / background load, wait until the
  // document is visible so the beacon is not dropped.
  function trackInitial() {
    if (
      document.visibilityState === "prerender" ||
      document.visibilityState === "hidden"
    ) {
      document.addEventListener(
        "visibilitychange",
        function once() {
          if (document.visibilityState === "visible") {
            document.removeEventListener("visibilitychange", once);
            pageview();
          }
        }
      );
    } else {
      pageview();
    }
  }

  trackInitial();

  // SPA navigations: pushState + back/forward. replaceState is only treated
  // as a navigation when the path/query actually changes (many apps call
  // replaceState on scroll to update the hash or UI state).
  var his = window.history;
  if (his.pushState) {
    var _push = his.pushState;
    var _replace = his.replaceState;
    his.pushState = function () {
      var ret = _push.apply(this, arguments);
      onNav();
      return ret;
    };
    his.replaceState = function () {
      var ret = _replace.apply(this, arguments);
      onNav();
      return ret;
    };
    window.addEventListener("popstate", onNav);
  }

  // Restore from bfcache: treat as a fresh pageview.
  window.addEventListener("pageshow", function (event) {
    if (event.persisted) {
      lastPath = null;
      pageview();
    }
  });

  function api(type, name, props) {
    if (type === "event" && name) send(name, props);
  }
  // Drain any calls queued before the script finished loading:
  //   window.stomatopod = window.stomatopod || []; stomatopod.push([...])
  var queued = window.stomatopod;
  window.stomatopod = api;
  window.stomatopod.l = true;
  if (Array.isArray(queued)) {
    for (var i = 0; i < queued.length; i++) {
      api.apply(null, queued[i]);
    }
  }
})();
