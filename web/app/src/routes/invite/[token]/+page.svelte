<script lang="ts">
	// Enrolment from an invite link (SPEC §5.5): choose a username, then a passkey or a
	// password with an authenticator app, then save the recovery codes.
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { callJson, messageOf } from '$lib/api/http';
	import { session } from '$lib/api/session.svelte';
	import type { SignedIn } from '$lib/api/types';
	import { createPasskey, deviceName, passkeysSupported } from '$lib/api/webauthn';
	import RecoveryCodes from '$lib/components/RecoveryCodes.svelte';

	type Step = 'checking' | 'invalid' | 'username' | 'method' | 'password' | 'totp' | 'codes';
	const token = $derived(page.params.token ?? '');
	let step = $state<Step>('checking');
	let invite = $state<{ purpose: string; display_name: string | null } | null>(null);
	let username = $state('');
	let enrol = '';
	let password = $state('');
	let confirm = $state('');
	let qr = $state<string | null>(null);
	let secret = $state('');
	let code = $state('');
	let codes = $state<string[]>([]);
	let signedIn: SignedIn | null = null;
	let busy = $state(false);
	let error = $state('');

	const json = (body: unknown) => ({ method: 'POST', body: JSON.stringify(body) });

	async function run(fn: () => Promise<void>) {
		busy = true;
		error = '';
		try {
			await fn();
		} catch (e) {
			error = messageOf(e);
		} finally {
			busy = false;
		}
	}

	$effect(() => {
		const t = token;
		void callJson<{ purpose: string; display_name: string | null }>('v1/auth/invite/inspect', json({ token: t }))
			.then((r) => {
				invite = r;
				// A reset keeps the username: go straight to choosing how to sign in.
				if (r.purpose === 'reset') void start('');
				else step = 'username';
			})
			.catch(() => (step = 'invalid'));
	});

	function start(name: string) {
		return run(async () => {
			const r = await callJson<{ enrol: string; username: string }>('v1/auth/enrol/start', json({ token, username: name }));
			enrol = r.enrol;
			username = r.username;
			step = 'method';
		});
	}

	const passkey = () =>
		run(async () => {
			const r = await callJson<{ options: Record<string, unknown> }>('v1/auth/enrol/passkey/start', json({ enrol }));
			const credential = await createPasskey(r.options);
			await callJson('v1/auth/enrol/passkey/finish', json({ enrol, name: deviceName(), credential }));
			await finish();
		});

	const setPassword = (e: Event) => {
		e.preventDefault();
		if (password !== confirm) {
			error = 'The passwords don’t match.';
			return;
		}
		return run(async () => {
			const r = await callJson<{ otpauth_uri: string; secret: string; qr_svg: string | null }>('v1/auth/enrol/password', json({ enrol, password }));
			// The QR code is an SVG drawn by the server, shown as an image (never as markup).
			qr = r.qr_svg ? `data:image/svg+xml;charset=utf-8,${encodeURIComponent(r.qr_svg)}` : null;
			secret = r.secret;
			password = confirm = '';
			step = 'totp';
		});
	};

	const confirmTotp = (e: Event) => {
		e.preventDefault();
		return run(async () => {
			await callJson('v1/auth/enrol/totp', json({ enrol, code: code.trim() }));
			await finish();
		});
	};

	async function finish() {
		const r = await callJson<SignedIn>('v1/auth/enrol/finish', json({ enrol, device_name: deviceName() }));
		signedIn = r;
		codes = r.recovery_codes ?? [];
		step = 'codes';
	}

	function done() {
		if (signedIn) session.accept(signedIn);
		void goto('#/', { replaceState: true });
	}
</script>

<main class="page narrow">
	<img class="logo" src="./favicon.svg" alt="" width="64" height="64" />
	{#if step === 'checking'}
		<p class="muted">Checking your invite…</p>
	{:else if step === 'invalid'}
		<h1>This invite doesn’t work</h1>
		<p>It may have been used already, or it has expired. Ask for a new one.</p>
	{:else if step === 'username'}
		<h1>Welcome{invite?.display_name ? `, ${invite.display_name}` : ''}</h1>
		<p>You’ve been invited to ABShare. Choose a username to sign in with.</p>
		<form onsubmit={(e) => { e.preventDefault(); void start(username); }}>
			<label class="field">Username
				<input bind:value={username} autocomplete="username" autocapitalize="none" spellcheck="false" required minlength="3" maxlength="32" pattern="[A-Za-z0-9][A-Za-z0-9._\-]*" />
			</label>
			<p class="muted small">3 to 32 letters, numbers, dots, dashes or underscores.</p>
			<button class="btn wide" disabled={busy}>Continue</button>
		</form>
	{:else if step === 'method'}
		<h1>How will you sign in?</h1>
		{#if passkeysSupported()}
			<p>A passkey is the easiest and safest: your phone or computer unlocks it with your face, fingerprint or PIN.</p>
			<button class="btn wide" onclick={passkey} disabled={busy}>Create a passkey</button>
		{/if}
		<button class="btn plain wide" onclick={() => (step = 'password')} disabled={busy}>Use a password and an authenticator app</button>
	{:else if step === 'password'}
		<h1>Choose a password</h1>
		<form onsubmit={setPassword}>
			<input type="text" value={username} autocomplete="username" hidden readonly />
			<label class="field">Password <input type="password" bind:value={password} autocomplete="new-password" required minlength="12" maxlength="1024" /></label>
			<label class="field">Password again <input type="password" bind:value={confirm} autocomplete="new-password" required minlength="12" maxlength="1024" /></label>
			<p class="muted small">At least 12 characters. A few unrelated words work well.</p>
			<button class="btn wide" disabled={busy}>Continue</button>
		</form>
	{:else if step === 'totp'}
		<h1>Add ABShare to your authenticator app</h1>
		<p>Scan this code with an app such as Google Authenticator, Microsoft Authenticator or 1Password.</p>
		{#if qr}<img class="qr" src={qr} alt="QR code for your authenticator app" width="220" height="220" />{/if}
		<p class="muted small">Can’t scan it? Enter this key instead: <code class="secret">{secret}</code></p>
		<form onsubmit={confirmTotp}>
			<label class="field">The 6-digit code it shows <input bind:value={code} inputmode="numeric" autocomplete="one-time-code" pattern="[0-9 ]*" required maxlength="8" /></label>
			<button class="btn wide" disabled={busy}>Confirm</button>
		</form>
	{:else if step === 'codes'}
		<h1>Save your recovery codes</h1>
		<p>If you lose your passkey or authenticator, each of these codes signs you in once. Keep them somewhere safe, like a password manager.</p>
		<RecoveryCodes {codes} />
		<button class="btn wide" onclick={done}>I’ve saved them</button>
	{/if}
	<p class="error" role="alert">{error}</p>
</main>

<style>
	.logo {
		margin: 2rem auto 1rem;
	}
	.wide {
		width: 100%;
		margin-bottom: 0.75rem;
	}
	.small {
		font-size: 0.85rem;
	}
	.qr {
		margin: 1rem auto;
		background: #fff;
		border-radius: var(--radius-sm);
	}
	.secret {
		word-break: break-all;
		user-select: all;
	}
</style>
