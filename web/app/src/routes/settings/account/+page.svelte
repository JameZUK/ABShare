<script lang="ts">
	// Account: passkeys, password, authenticator app, recovery codes, and signing out.
	import { goto } from '$app/navigation';
	import { messageOf } from '$lib/api/http';
	import { api, post, session } from '$lib/api/session.svelte';
	import type { Me, PasskeyInfo } from '$lib/api/types';
	import { createPasskey, deviceName, passkeysSupported } from '$lib/api/webauthn';
	import { date } from '$lib/format';
	import { downloads } from '$lib/offline/downloads.svelte';
	import { player } from '$lib/player/player.svelte';
	import { withStepUp } from '$lib/stepup.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import RecoveryCodes from '$lib/components/RecoveryCodes.svelte';
	import Sheet from '$lib/components/Sheet.svelte';
	import StepUpSheet from '$lib/components/StepUpSheet.svelte';

	let me = $state<Me | null>(null);
	let passkeys = $state<PasskeyInfo[]>([]);
	let error = $state('');
	let note = $state('');
	let codes = $state<string[]>([]);
	let pwOpen = $state(false);
	let newPw = $state('');
	let newPw2 = $state('');
	let totpOpen = $state(false);
	let totp = $state<{ challenge: string; qr: string | null; secret: string } | null>(null);
	let totpCode = $state('');
	let outOpen = $state(false);
	let busy = $state(false);

	async function reload() {
		try {
			[me, passkeys] = await Promise.all([api<Me>('v1/me'), api<{ passkeys: PasskeyInfo[] }>('v1/me/passkeys').then((r) => r.passkeys)]);
			// A new factor lifts the limit after a recovery-code sign-in: let the rest of the app in.
			if (session.user?.needs_factor && !me.needs_factor) session.user = { ...session.user, needs_factor: false };
		} catch (e) {
			error = messageOf(e);
		}
	}
	$effect(() => {
		if (session.status === 'signed-in') void reload();
	});

	async function run(fn: () => Promise<void>, done?: string) {
		busy = true;
		error = note = '';
		try {
			await withStepUp(fn);
			if (done) note = done;
			await reload();
		} catch (e) {
			error = messageOf(e);
		} finally {
			busy = false;
		}
	}

	const addPasskey = () =>
		run(async () => {
			const s = await post<{ challenge: string; options: Record<string, unknown> }>('v1/me/passkeys/start', {});
			const credential = await createPasskey(s.options);
			await post('v1/me/passkeys/finish', { challenge: s.challenge, name: deviceName(), credential });
		}, 'Passkey added.');

	const removePasskey = (id: number) => run(() => api(`v1/me/passkeys/${id}`, { method: 'DELETE' }), 'Passkey removed.');

	const changePassword = (e: Event) => {
		e.preventDefault();
		if (newPw !== newPw2) {
			error = 'The passwords don’t match.';
			return;
		}
		return run(async () => {
			await post('v1/me/password', { password: newPw });
			newPw = newPw2 = '';
			pwOpen = false;
		}, 'Password changed. Your other devices have been signed out.');
	};

	const startTotp = () =>
		run(async () => {
			const r = await post<{ challenge: string; qr_svg: string | null; secret: string }>('v1/me/totp/start', {});
			totp = { challenge: r.challenge, secret: r.secret, qr: r.qr_svg ? `data:image/svg+xml;charset=utf-8,${encodeURIComponent(r.qr_svg)}` : null };
			totpOpen = true;
		});

	const finishTotp = (e: Event) => {
		e.preventDefault();
		return run(async () => {
			await post('v1/me/totp/finish', { challenge: totp?.challenge, code: totpCode.trim() });
			totpOpen = false;
			totp = null;
			totpCode = '';
		}, 'Authenticator app added.');
	};

	const removeTotp = () => run(() => api('v1/me/totp', { method: 'DELETE' }), 'Authenticator app removed.');

	const newCodes = () =>
		run(async () => {
			codes = (await post<{ recovery_codes: string[] }>('v1/me/recovery', {})).recovery_codes;
		});

	async function signOut(remove: boolean) {
		const user = session.user?.username;
		outOpen = false;
		player.close();
		await session.signOut(remove);
		if (remove && user) await downloads.removeAll(user);
		void goto('#/', { replaceState: true });
	}
</script>

<main class="page narrow">
	<button class="icon-btn" onclick={() => history.back()} aria-label="Back"><Icon name="back" /></button>
	<h1>Account</h1>
	{#if session.user?.needs_factor}
		<p class="notice" role="alert">You signed in with a recovery code. Set up a new passkey or authenticator app below to carry on. If a lost phone or computer holds one of your passkeys, remove it too.</p>
	{/if}
	{#if me}<p class="muted">Signed in as <strong>{me.username}</strong>{me.display_name !== me.username ? ` (${me.display_name})` : ''}</p>{/if}
	{#if note}<p role="status">{note}</p>{/if}
	{#if error}<p class="error" role="alert">{error}</p>{/if}

	<section>
		<h2>Passkeys</h2>
		{#each passkeys as k (k.id)}
			<div class="item row">
				<Icon name="key" />
				<span class="spacer"><strong>{k.name}</strong><br /><span class="muted small">Added {date(k.created_at)}{k.last_used ? ` · last used ${date(k.last_used)}` : ''}</span></span>
				<button class="icon-btn" onclick={() => removePasskey(k.id)} aria-label="Remove passkey {k.name}" disabled={busy}><Icon name="trash" /></button>
			</div>
		{:else}
			<p class="muted">No passkeys yet.</p>
		{/each}
		{#if passkeysSupported()}<button class="btn soft" onclick={addPasskey} disabled={busy}>Add a passkey</button>{/if}
	</section>

	<section>
		<h2>Password and authenticator</h2>
		{#if me?.factors.password}
			<button class="btn plain" onclick={() => (pwOpen = true)} disabled={busy}>Change password</button>
		{/if}
		{#if me?.factors.totp}
			<p>An authenticator app is set up.</p>
			<button class="btn plain danger" onclick={removeTotp} disabled={busy}>Remove authenticator app</button>
		{:else if me?.factors.password}
			<button class="btn soft" onclick={startTotp} disabled={busy}>Set up an authenticator app</button>
		{/if}
	</section>

	<section>
		<h2>Recovery codes</h2>
		<p class="muted">{me?.factors.recovery_codes_left ?? 0} unused. Each signs you in once if you lose your other ways in.</p>
		{#if codes.length}
			<p>Your new codes (the old ones no longer work):</p>
			<RecoveryCodes {codes} />
		{:else}
			<button class="btn plain" onclick={newCodes} disabled={busy}>Make new codes</button>
		{/if}
	</section>

	<section>
		<button class="btn plain" onclick={() => (outOpen = true)}><Icon name="signout" /> Sign out</button>
	</section>
</main>

<StepUpSheet {me} />

<Sheet bind:open={pwOpen} title="Change password">
	<form onsubmit={changePassword}>
		<input type="text" value={me?.username ?? ''} autocomplete="username" hidden readonly />
		<label class="field">New password <input type="password" bind:value={newPw} autocomplete="new-password" minlength="12" maxlength="1024" required /></label>
		<label class="field">New password again <input type="password" bind:value={newPw2} autocomplete="new-password" minlength="12" maxlength="1024" required /></label>
		<p class="muted small">Your other devices will be signed out.</p>
		<button class="btn" disabled={busy}>Change password</button>
	</form>
</Sheet>

<Sheet bind:open={totpOpen} title="Authenticator app">
	{#if totp}
		<p>Scan this with your authenticator app, then enter the code it shows.</p>
		{#if totp.qr}<img class="qr" src={totp.qr} alt="QR code for your authenticator app" width="200" height="200" />{/if}
		<p class="muted small">Or enter this key: <code class="secret">{totp.secret}</code></p>
		<form onsubmit={finishTotp}>
			<label class="field">Code <input bind:value={totpCode} inputmode="numeric" autocomplete="one-time-code" required maxlength="8" /></label>
			<button class="btn" disabled={busy}>Confirm</button>
		</form>
	{/if}
</Sheet>

<Sheet bind:open={outOpen} title="Sign out">
	<p>Keep your downloaded books on this device for next time?</p>
	<div class="choices">
		<button class="btn" onclick={() => signOut(false)}>Keep downloads</button>
		<button class="btn plain danger" onclick={() => signOut(true)}>Remove downloads</button>
	</div>
</Sheet>

<style>
	.notice {
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
		padding: 0.75rem 1rem;
		border-radius: var(--radius-sm);
	}
	section {
		margin: 1.5rem 0;
	}
	.item {
		min-height: 56px;
		border-bottom: 1px solid var(--line);
		margin-bottom: 0.5rem;
	}
	.small {
		font-size: 0.85rem;
	}
	.danger {
		color: var(--danger);
	}
	.qr {
		margin: 0.5rem auto;
		background: #fff;
		border-radius: var(--radius-sm);
	}
	.secret {
		word-break: break-all;
		user-select: all;
	}
	.choices {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
	}
	section .btn {
		margin: 0.25rem 0.25rem 0.25rem 0;
	}
</style>
