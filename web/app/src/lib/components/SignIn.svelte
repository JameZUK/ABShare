<script lang="ts">
	// Sign-in: a passkey first (phishing-resistant, one tap), or a password followed by an
	// authenticator code or a recovery code.
	import { callJson, messageOf } from '$lib/api/http';
	import { session } from '$lib/api/session.svelte';
	import type { SignedIn } from '$lib/api/types';
	import { deviceName, getPasskey, passkeysSupported } from '$lib/api/webauthn';

	type Step = 'start' | 'password' | 'code';
	let step = $state<Step>(passkeysSupported() ? 'start' : 'password');
	let username = $state('');
	let password = $state('');
	let code = $state('');
	let useRecovery = $state(false);
	let challenge = '';
	let methods = $state<string[]>([]);
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

	const withPasskey = () =>
		run(async () => {
			const start = await callJson<{ challenge: string; options: Record<string, unknown> }>('v1/auth/login/passkey/start', json({}));
			const credential = await getPasskey(start.options);
			session.accept(await callJson<SignedIn>('v1/auth/login/passkey/finish', json({ challenge: start.challenge, device_name: deviceName(), credential })));
		});

	const withPassword = (e: Event) => {
		e.preventDefault();
		return run(async () => {
			const r = await callJson<{ challenge: string; methods: string[] }>('v1/auth/login/password', json({ username, password, device_name: deviceName() }));
			challenge = r.challenge;
			methods = r.methods;
			useRecovery = !r.methods.includes('totp');
			password = '';
			step = 'code';
		});
	};

	const withCode = (e: Event) => {
		e.preventDefault();
		return run(async () => {
			const path = useRecovery ? 'v1/auth/login/recovery' : 'v1/auth/login/totp';
			try {
				session.accept(await callJson<SignedIn>(path, json({ challenge, code: code.trim() })));
			} catch (err) {
				// The challenge is single use: a wrong code means starting again.
				step = 'password';
				code = '';
				throw err;
			}
		});
	};
</script>

<main class="page narrow signin">
	<img class="logo" src="./favicon.svg" alt="" width="72" height="72" />
	<h1>ABShare</h1>
	<p class="tagline serif">Your audiobooks, wherever you are.</p>

	{#if step === 'start'}
		<button class="btn wide" onclick={withPasskey} disabled={busy}>Sign in with a passkey</button>
		<button class="btn soft wide" onclick={() => (step = 'password')} disabled={busy}>Use a password</button>
	{:else if step === 'password'}
		<form onsubmit={withPassword}>
			<label class="field">Username <input bind:value={username} autocomplete="username" autocapitalize="none" spellcheck="false" required maxlength="32" /></label>
			<label class="field">Password <input type="password" bind:value={password} autocomplete="current-password" required maxlength="1024" /></label>
			<button class="btn wide" disabled={busy}>Continue</button>
		</form>
		{#if passkeysSupported()}
			<button class="btn soft wide" onclick={() => (step = 'start')} disabled={busy}>Use a passkey instead</button>
		{/if}
	{:else}
		<form onsubmit={withCode}>
			{#if useRecovery}
				<label class="field">Recovery code <input bind:value={code} autocomplete="off" autocapitalize="characters" spellcheck="false" required maxlength="40" /></label>
			{:else}
				<label class="field">Code from your authenticator app <input bind:value={code} inputmode="numeric" autocomplete="one-time-code" pattern="[0-9 ]*" required maxlength="8" /></label>
			{/if}
			<button class="btn wide" disabled={busy}>Sign in</button>
		</form>
		{#if methods.includes('totp')}
			<button class="btn soft wide" onclick={() => { useRecovery = !useRecovery; code = ''; }}>{useRecovery ? 'Use an authenticator code' : 'Use a recovery code'}</button>
		{/if}
	{/if}

	<p class="error" role="alert">{error}</p>
	<p class="muted small foot">ABShare is invite-only. Ask the person who runs it for an invite link.</p>
</main>

<style>
	.signin {
		max-width: 400px;
		padding-top: max(10vh, 3rem);
		text-align: center;
	}
	.signin form {
		text-align: left;
	}
	.logo {
		margin: 0 auto 1.25rem;
		border-radius: 18px;
		box-shadow: var(--shadow-book);
	}
	h1 {
		font-size: 2.6rem;
		margin-bottom: 0.2rem;
	}
	.tagline {
		font-style: italic;
		font-size: 1.15rem;
		color: var(--fg-muted);
		margin-bottom: 2.25rem;
	}
	.wide {
		width: 100%;
		margin-bottom: 0.75rem;
		min-height: 50px;
	}
	.small {
		font-size: 0.85rem;
	}
	.foot {
		margin-top: 1.5rem;
		padding-top: 1.25rem;
		border-top: 1px solid var(--line);
	}
</style>
