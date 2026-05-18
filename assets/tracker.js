(function () {
  'use strict';

  if (navigator.doNotTrack === '1' || window.doNotTrack === '1') return;

  var script = document.currentScript || document.querySelector('script[data-site]');
  if (!script || script.hasAttribute('data-exclude')) return;

  var endpoint = script.getAttribute('data-api') || '/api/v1/event';
  var siteKey = script.getAttribute('data-site');
  if (!siteKey) return;

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
      navigator.sendBeacon(endpoint, new Blob([payload], { type: 'application/json' }));
    } else {
      fetch(endpoint, { method: 'POST', body: payload, keepalive: true, headers: { 'Content-Type': 'application/json' } });
    }
  }

  send('pageview');

  var _push = history.pushState;
  var _replace = history.replaceState;
  function onNav() { send('pageview'); }
  history.pushState = function () { _push.apply(this, arguments); onNav(); };
  history.replaceState = function () { _replace.apply(this, arguments); onNav(); };
  window.addEventListener('popstate', onNav);

  window.stomatopod = function (type, name, props) {
    if (type === 'event') send(name, props);
  };
})();
