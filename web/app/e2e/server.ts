// Starts a fresh ABShare for the browser tests: the invented showcase library, read-only, a
// new data directory, a scan with the real jailed ffmpeg, one admin invite, and `serve`.
// Nothing here touches a real library. Binaries come from ABSHARE_BIN_DIR (default
// target/debug); the built web app from web/app/build. Commands run with argument arrays,
// never through a shell.
import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

export const PORT = 8744;
export const ORIGIN = `http://localhost:${PORT}`;
const ROOT = path.resolve(import.meta.dirname, '../../..');
export const STATE_DIR = path.join(import.meta.dirname, '.state');

let server: ChildProcess | undefined;

function bin(name: string) {
	return path.join(process.env.ABSHARE_BIN_DIR ?? path.join(ROOT, 'target/debug'), name);
}

/** Runs a program to completion; throws with its output if it fails. */
function run(cmd: string, args: string[]): string {
	const r = spawnSync(cmd, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
	if (r.status !== 0) throw new Error(`${cmd} ${args.join(' ')} failed (${r.status ?? r.signal}):\n${r.stdout}\n${r.stderr}`);
	return r.stdout;
}

export default async function setup() {
	for (const b of ['abshare', 'abshare-jail']) if (!fs.existsSync(bin(b))) throw new Error(`build ${bin(b)} first (cargo build)`);
	if (!fs.existsSync(path.join(ROOT, 'web/app/build/index.html'))) throw new Error('build the web app first (npm run build)');

	const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'abshare-e2e-'));
	const lib = path.join(dir, 'lib');
	run(path.join(ROOT, 'tools/demo/make-showcase.sh'), [lib]);
	run('chmod', ['-R', 'a-w', lib]);
	fs.mkdirSync(path.join(dir, 'data'), { mode: 0o700 });
	const config = path.join(dir, 'abshare.toml');
	fs.writeFileSync(
		config,
		`listen = "127.0.0.1:${PORT}"
data_dir = "${dir}/data"
public_origin = "${ORIGIN}"
web_dir = "${ROOT}/web/app/build"

[ffmpeg]
ffmpeg = "/usr/bin/ffmpeg"
ffprobe = "/usr/bin/ffprobe"
jail = "${bin('abshare-jail')}"
libs = ["/usr/lib"]

[[library]]
name = "Audiobooks"
path = "${lib}"
`,
		{ mode: 0o600 },
	);

	run(bin('abshare'), ['scan', '--config', config]);
	const out = run(bin('abshare'), ['admin', 'invite', '--config', config, '--admin', '--name', 'Ada Lovelace']);
	const invite = /https?:\/\/\S+#\/invite\/[0-9a-f]+/.exec(out)?.[0];
	if (!invite) throw new Error(`no invite link in: ${out}`);

	server = spawn(bin('abshare'), ['serve', '--config', config], { stdio: ['ignore', 'inherit', 'inherit'] });
	const deadline = Date.now() + 30_000;
	for (;;) {
		try {
			if ((await fetch(`${ORIGIN}/healthz`)).ok) break;
		} catch {
			/* not up yet */
		}
		if (Date.now() > deadline) throw new Error('the server did not start');
		await new Promise((r) => setTimeout(r, 200));
	}

	fs.rmSync(STATE_DIR, { recursive: true, force: true });
	fs.mkdirSync(STATE_DIR, { recursive: true });
	fs.writeFileSync(path.join(STATE_DIR, 'invite.txt'), invite);

	return async () => {
		server?.kill('SIGTERM');
		run('chmod', ['-R', 'u+w', lib]);
		fs.rmSync(dir, { recursive: true, force: true });
	};
}
