// Library metadata comes from untrusted files: the app must never parse strings as HTML.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const ROOTS = ['src'].map((d) => new URL(`../../${d}/`, import.meta.url).pathname);
const BANNED = [
	[/\{@html\b/, '{@html}'],
	[/\.\s*(?:innerHTML|outerHTML)\s*=/, 'innerHTML/outerHTML assignment'],
	[/insertAdjacentHTML\s*\(/, 'insertAdjacentHTML'],
	[/document\.write\s*\(/, 'document.write'],
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
		.filter((d) => d.isFile() && /\.(svelte|ts|js|mjs)$/.test(d.name) && !d.parentPath.includes('/pkg'))
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
	for (const s of ['{@html x}', 'el.innerHTML = s', 'a.insertAdjacentHTML("x", y)', 'eval(x)', 'new Function("x")', '<iframe srcdoc=x>', 'new DOMParser()', 'el.setAttribute("onclick", f)', "setTimeout('x()', 1)"]) {
		assert.ok(BANNED.some(([re]) => re.test(s)), s);
	}
});
