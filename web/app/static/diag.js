// Start-up diagnostics. Runs before the app. If the page is still blank a few seconds after
// loading, or the address has ?diag, it shows what it knows as plain text: script errors, the
// screen, whether styles loaded and whether the app drew anything. It sends nothing anywhere;
// it is for a photo of the screen.
(function () {
	'use strict';
	var errors = [];
	var started = Date.now();
	function note(text) {
		if (errors.length < 8) errors.push(String(text).slice(0, 300));
	}
	addEventListener('error', function (e) {
		var t = e.target;
		if (t && t !== window && (t.src || t.href)) note('Could not load ' + (t.src || t.href));
		else note((e.message || 'Error') + (e.filename ? ' (' + e.filename.split('/').pop() + ':' + e.lineno + ')' : ''));
	}, true);
	addEventListener('unhandledrejection', function (e) {
		var r = e.reason;
		note('Unhandled: ' + (r && (r.stack || r.message) ? (r.message || '') + ' ' + String(r.stack || '').split('\n').slice(0, 2).join(' ') : String(r)));
	});

	// Every probe is guarded: this must work in exactly the browsers where things go wrong.
	function safe(fn, fallback) {
		try {
			var v = fn();
			return v === undefined || v === null ? fallback : v;
		} catch (e) {
			return fallback;
		}
	}

	function report(forced) {
		var text = safe(function () {
			return document.body.innerText.trim().length;
		}, 0);
		if (!forced && text > 0) return false;
		var drawn = safe(function () {
			return document.getElementById('app').querySelectorAll('main, nav, form, button, h1').length;
		}, 0);
		var lines = [
			forced ? 'ABShare diagnostics' : 'ABShare could not start on this browser.',
			'Please send a photo of this screen to whoever invited you. Tap to close.',
			'',
			'Browser: ' + safe(function () { return navigator.userAgent; }, 'unknown'),
			'Screen: ' + safe(function () { return innerWidth + '×' + innerHeight + ' at ' + (window.devicePixelRatio || 1) + '×'; }, 'unknown'),
			'Dark mode: ' + safe(function () { return matchMedia('(prefers-color-scheme: dark)').matches ? 'yes' : 'no'; }, 'unknown'),
			'Styles loaded: ' + safe(function () { return document.styleSheets.length; }, 'unknown'),
			'App drew: ' + drawn + ' elements, ' + text + ' characters of text',
			'Colours: ' + safe(function () { var b = getComputedStyle(document.body); return b.color + ' on ' + b.backgroundColor; }, 'unknown'),
			'Cookies: ' + safe(function () { return navigator.cookieEnabled ? 'allowed' : 'BLOCKED'; }, 'unknown'),
			'Seconds since loading: ' + Math.round((Date.now() - started) / 1000),
			'',
			errors.length ? 'Errors:' : 'No script errors.',
		].concat(errors);
		var box = document.createElement('pre');
		box.setAttribute('role', 'alert');
		box.style.cssText =
			'position:fixed;inset:0;z-index:2147483647;margin:0;padding:16px;overflow:auto;' +
			'background:#fff;color:#000;font:14px/1.4 monospace;white-space:pre-wrap;word-break:break-word';
		box.textContent = lines.join('\n');
		box.addEventListener('click', function () {
			box.remove();
		});
		document.documentElement.appendChild(box);
		return true;
	}

	var forced = /[?&]diag\b/.test(location.search);
	setTimeout(function () {
		report(forced);
	}, forced ? 4000 : 8000);
})();
