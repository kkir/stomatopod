(function () {
  "use strict";

  if (navigator.doNotTrack === "1" || window.doNotTrack === "1") return;

  var script =
    document.currentScript || document.querySelector("script[data-site]");
  if (!script || script.hasAttribute("data-exclude")) return;

  var endpoint = script.getAttribute("data-api") || "/api/v1/event";
  var siteKey = script.getAttribute("data-site");
  if (!siteKey) return;

  // Opt-outs: a site can disable the higher-volume Tier-4 collectors with
  // data attributes (data-no-vitals / data-no-scroll / data-no-clicks /
  // data-no-search) while keeping pageview analytics.
  function enabled(name) {
    return !script.hasAttribute("data-no-" + name);
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
    if (navigator.sendBeacon) {
      navigator.sendBeacon(
        endpoint,
        new Blob([payload], { type: "application/json" }),
      );
    } else {
      fetch(endpoint, {
        method: "POST",
        body: payload,
        keepalive: true,
        headers: { "Content-Type": "application/json" },
      });
    }
  }

  send("pageview");

  // ---- Site search auto-detection (__search__) ----
  // Detect common search params on each pageview and report the query.
  var SEARCH_PARAMS = ["q", "query", "s", "search", "term", "keyword"];

  function trackSearch() {
    if (!enabled("search")) return;
    try {
      var params = new URLSearchParams(location.search);
      for (var i = 0; i < SEARCH_PARAMS.length; i++) {
        if (params.has(SEARCH_PARAMS[i])) {
          var q = params.get(SEARCH_PARAMS[i]);
          if (q) {
            send("__search__", {
              query: String(q).slice(0, 200),
              url: location.pathname,
            });
          }
          return;
        }
      }
    } catch (e) {}
  }
  trackSearch();

  // ---- Scroll depth milestones (__scroll__) ----
  var milestones = [25, 50, 75, 100];
  var reached = {};

  function scrollPct() {
    var el = document.documentElement;
    var h = el.scrollHeight;
    if (!h) return 0;
    return Math.round(((window.scrollY + window.innerHeight) / h) * 100);
  }

  function onScroll() {
    if (!enabled("scroll")) return;
    var pct = scrollPct();
    for (var i = 0; i < milestones.length; i++) {
      var m = milestones[i];
      if (pct >= m && !reached[m]) {
        reached[m] = true;
        send("__scroll__", { depth: m, url: location.pathname });
      }
    }
  }
  if (enabled("scroll")) {
    window.addEventListener("scroll", onScroll, { passive: true });
  }

  // ---- Click heatmap (__click__) ----
  // Coordinates as % of viewport width / full page height. Form fields are
  // excluded for privacy (no text-input position capture).
  function onClick(e) {
    if (!enabled("clicks")) return;
    try {
      var t = e.target || {};
      var tag = (t.tagName || "").toUpperCase();
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
      var docH = document.documentElement.scrollHeight || 1;
      send("__click__", {
        x: Math.round((e.clientX / (window.innerWidth || 1)) * 100),
        y: Math.round(((e.clientY + window.scrollY) / docH) * 100),
        url: location.pathname,
        element: tag + (t.id ? "#" + t.id : ""),
      });
    } catch (err) {}
  }
  if (enabled("clicks")) {
    document.addEventListener("click", onClick, { passive: true });
  }

  // ---- Core Web Vitals (__vital__) ----
  // Native PerformanceObserver collection (no external library). LCP and CLS
  // are flushed on page hide; INP is approximated from event-timing durations.
  function rate(metric, value) {
    if (metric === "LCP")
      return value <= 2500 ? "good" : value <= 4000 ? "needs-improvement" : "poor";
    if (metric === "CLS")
      return value <= 0.1 ? "good" : value <= 0.25 ? "needs-improvement" : "poor";
    if (metric === "INP")
      return value <= 200 ? "good" : value <= 500 ? "needs-improvement" : "poor";
    return "needs-improvement";
  }

  function sendVital(metric, value) {
    send("__vital__", {
      metric: metric,
      value: Math.round(value * 1000) / 1000,
      rating: rate(metric, value),
      url: location.pathname,
    });
  }

  function collectVitals() {
    if (!enabled("vitals") || typeof PerformanceObserver === "undefined") return;
    var lcp = 0;
    var cls = 0;
    var inp = 0;
    var sent = {};

    function obs(type, cb, opts) {
      try {
        var o = new PerformanceObserver(cb);
        o.observe(Object.assign({ type: type, buffered: true }, opts || {}));
        return o;
      } catch (e) {
        return null;
      }
    }

    obs("largest-contentful-paint", function (list) {
      var entries = list.getEntries();
      var last = entries[entries.length - 1];
      if (last) lcp = last.renderTime || last.loadTime || last.startTime;
    });

    obs("layout-shift", function (list) {
      list.getEntries().forEach(function (entry) {
        if (!entry.hadRecentInput) cls += entry.value;
      });
    });

    obs(
      "event",
      function (list) {
        list.getEntries().forEach(function (entry) {
          if (entry.duration > inp) inp = entry.duration;
        });
      },
      { durationThreshold: 40 },
    );

    function flush() {
      if (lcp && !sent.LCP) {
        sent.LCP = true;
        sendVital("LCP", lcp);
      }
      if (!sent.CLS) {
        sent.CLS = true;
        sendVital("CLS", cls);
      }
      if (inp && !sent.INP) {
        sent.INP = true;
        sendVital("INP", inp);
      }
    }

    addEventListener("visibilitychange", function () {
      if (document.visibilityState === "hidden") flush();
    });
    addEventListener("pagehide", flush);
  }
  collectVitals();

  var _push = history.pushState;
  var _replace = history.replaceState;
  function onNav() {
    // Reset scroll milestones for the new route (SPA navigation).
    reached = {};
    send("pageview");
    trackSearch();
  }
  history.pushState = function () {
    _push.apply(this, arguments);
    onNav();
  };
  history.replaceState = function () {
    _replace.apply(this, arguments);
    onNav();
  };
  window.addEventListener("popstate", onNav);

  window.stomatopod = function (type, name, props) {
    if (type === "event") send(name, props);
  };
})();
