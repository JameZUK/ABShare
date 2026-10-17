// Start-up diagnostics. Runs before the app. If the page is still blank a few seconds after
// loading, or the address has ?diag, it shows what it knows as plain text (script errors, the
// screen, whether styles loaded and whether the app drew anything) and sends the same text to
// the server, where admins can read it (POST /v1/diag; Admin → Overview).
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
		// The page may hold text no one can see: check where the main heading is and its colours.
		var heading = safe(function () {
			var h = document.querySelector('#app h1, #app main, #app button');
			if (!h) return null;
			var r = h.getBoundingClientRect();
			var c = getComputedStyle(h);
			var bg = 'rgba(0, 0, 0, 0)';
			for (var e = h; e && bg === 'rgba(0, 0, 0, 0)'; e = e.parentElement) bg = getComputedStyle(e).backgroundColor;
			if (bg === 'rgba(0, 0, 0, 0)') bg = 'rgb(255, 255, 255)';
			var hidden = r.width < 1 || r.height < 1 || r.bottom <= 0 || r.right <= 0 || r.top >= innerHeight || r.left >= innerWidth ||
				c.color === bg || c.visibility === 'hidden' || c.opacity === '0';
			return {
				hidden: hidden,
				line: h.tagName.toLowerCase() + ' at ' + Math.round(r.left) + ',' + Math.round(r.top) + ' size ' + Math.round(r.width) + '×' + Math.round(r.height) +
					', colour ' + c.color + ' on ' + bg + ', opacity ' + c.opacity + ', visibility ' + c.visibility,
			};
		}, null);
		if (!forced && text > 0 && !(heading && heading.hidden)) return false;
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
			'Heading: ' + (heading ? heading.line + (heading.hidden ? ' (NOT VISIBLE)' : '') : 'none'),
			'Root: ' + safe(function () { var d = document.documentElement; var c = getComputedStyle(d); return 'mode ' + (d.dataset.mode || '?') + ', theme ' + (d.dataset.theme || '?') + ', background ' + c.backgroundColor + ', page ' + d.scrollWidth + '×' + d.scrollHeight; }, 'unknown'),
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
		send(lines);
		return true;
	}

	// Also send the report to the server, where an admin can read it (Admin → Overview). Same
	// origin only; once per page.
	var sent = false;
	function send(lines) {
		if (sent) return;
		sent = true;
		safe(function () {
			fetch(new URL('./v1/diag', document.baseURI).href, {
				method: 'POST',
				headers: { 'content-type': 'application/json', 'x-abshare-client': '1' },
				body: JSON.stringify({ lines: lines.filter(Boolean).slice(0, 40) }),
				keepalive: true,
				credentials: 'omit',
			}).catch(function () {});
		}, null);
	}

	var forced = /[?&]diag\b/.test(location.search);
	setTimeout(function () {
		report(forced);
	}, forced ? 4000 : 8000);
})();
