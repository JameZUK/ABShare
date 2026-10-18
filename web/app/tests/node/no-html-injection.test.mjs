// Library metadata comes from untrusted files: the app must never parse strings as HTML.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const ROOTS = ['src', 'static'].map((d) => new URL(`../../${d}/`, import.meta.url).pathname);
const BANNED = [
	[/\{@html\b/, '{@html}'],
	[/\.\s*(?:innerHTML|outerHTML)\s*\+?=/, 'innerHTML/outerHTML assignment'],
	[/\[\s*['"`](?:innerHTML|outerHTML)['"`]\s*\]/, 'innerHTML/outerHTML by name'],
	[/\b(?:innerHTML|outerHTML)\s*:/, 'innerHTML/outerHTML in an object (Object.assign)'],
	[/setHTMLUnsafe|parseHTMLUnsafe/, 'setHTMLUnsafe/parseHTMLUnsafe'],
	[/insertAdjacentHTML\s*\(/, 'insertAdjacentHTML'],
	[/document\.write(?:ln)?\s*\(/, 'document.write/writeln'],
	[/\beval\s*\(/, 'eval'],
	[/new\s+Function\s*\(/, 'new Function'],
	[/createContextualFragment\s*\(/, 'createContextualFragment'],
	[/\bsrcdoc\b/, 'srcdoc'],
	[/\bDOMParser\b/, 'DOMParser'],
	[/setAttribute\s*\(\s*['"`]on/i, "setAttribute('on…')"],
	[/set(?:Timeout|Interval)\s*\(\s*['"`]/, 'string timer'],
];

function files(dir) {
	return readdirSync(dir, { recursive: true, withFileTypes: true })
		.filter((d) => d.isFile() && /\.(svelte|ts|js|mjs|html)$/.test(d.name) && !d.parentPath.includes('/pkg'))
		.map((d) => join(d.parentPath, d.name));
}

test('no HTML-parsing APIs in the app', () => {
	const found = [];
	for (const root of ROOTS) {
		for (const f of files(root)) {
			const code = readFileSync(f, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
			for (const [re, what] of BANNED) if (re.test(code)) found.push(`${f}: ${what}`);
		}
	}
	assert.deepEqual(found, []);
});

test('the check itself catches each pattern', () => {
	for (const s of ['el.innerHTML += s', "el['innerHTML'] = s", 'Object.assign(el, { innerHTML: s })', 'el.setHTMLUnsafe(s)', 'document.writeln(s)', '{@html x}', 'el.innerHTML = s', 'a.insertAdjacentHTML("x", y)', 'eval(x)', 'new Function("x")', '<iframe srcdoc=x>', 'new DOMParser()', 'el.setAttribute("onclick", f)', "setTimeout('x()', 1)"]) {
		assert.ok(BANNED.some(([re]) => re.test(s)), s);
	}
});

// Every dynamic href and src in a component must start with a fixed, safe prefix, so server
// data can never become a javascript: URL or point elsewhere (review W4).
test('dynamic links and sources start with a safe literal', () => {
	const bad = [];
	const safe = /^(?:#\/|v1\/|\.\/|data:image\/svg\+xml;|blob:)/;
	for (const root of ROOTS) {
		for (const f of files(root).filter((f) => f.endsWith('.svelte'))) {
			const code = readFileSync(f, 'utf8');
			for (const m of code.matchAll(/(?<![\w:-])(href|src|action|formaction|poster|xlink:href)=(\{[^}]*\}|"[^"]*\{[^"]*")/g)) {
				const v = m[2];
				// {url}-style values are checked where they're built; template strings must
				// start with a safe literal.
				if (v.startsWith('"')) {
					if (!safe.test(v.slice(1))) bad.push(`${f}: ${m[0]}`);
				} else if (v.startsWith('{`') && !safe.test(v.slice(2))) {
					bad.push(`${f}: ${m[0]}`);
				}
			}
		}
	}
	assert.deepEqual(bad, []);
});

// Inline styles built from data could carry CSS from the server (url(), expressions); use
// `style:prop={value}` directives, which set one property each (review W16).
const DYNAMIC_STYLE = /\sstyle=(?:"[^"]*\{|'[^']*\{|\{)/;

test('no inline style attributes built from data', () => {
	const bad = [];
	for (const root of ROOTS) {
		for (const f of files(root).filter((f) => f.endsWith('.svelte'))) {
			const code = readFileSync(f, 'utf8');
			if (DYNAMIC_STYLE.test(code)) bad.push(f);
		}
	}
	assert.deepEqual(bad, []);
});

test('the style check catches each form and allows directives', () => {
	for (const s of ['<div style="color: {c}">', '<div style={css}>', "<div style='width:{w}px'>"]) assert.ok(DYNAMIC_STYLE.test(s), s);
	for (const s of ['<div style:width="{w}px">', '<div style:--h={hue}>', '<div style="color: red">']) assert.ok(!DYNAMIC_STYLE.test(s), s);
});
