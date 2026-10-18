<script lang="ts">
	import { messageOf } from '$lib/api/http';
	import { post } from '$lib/api/session.svelte';
	import type { Me } from '$lib/api/types';
	import { getPasskey, passkeysSupported } from '$lib/api/webauthn';
	import { stepUp } from '$lib/stepup.svelte';
	import Sheet from './Sheet.svelte';

	let { me }: { me: Me | null } = $props();
	let password = $state('');
	let code = $state('');
	let busy = $state(false);
	let error = $state('');
	let open = $state(false);

	$effect(() => {
		open = stepUp.open;
	});
	$effect(() => {
		if (!open && stepUp.open) stepUp.done(false);
	});

	async function run(fn: () => Promise<void>) {
		busy = true;
		error = '';
		try {
			await fn();
			password = code = '';
			stepUp.done(true);
		} catch (e) {
			error = messageOf(e);
		} finally {
			busy = false;
		}
	}

	const withPasskey = () =>
		run(async () => {
			const s = await post<{ challenge: string; options: Record<string, unknown> }>('v1/auth/step-up/passkey/start', {});
			const credential = await getPasskey(s.options);
			await post('v1/auth/step-up/passkey/finish', { challenge: s.challenge, credential });
		});

	const withPassword = (e: Event) => {
		e.preventDefault();
		return run(async () => {
			await post('v1/auth/step-up/totp', { password, code: code.trim() });
		});
	};
</script>

<Sheet bind:open title="Confirm it’s you">
	<p class="muted">For your security, confirm it’s you before making this change.</p>
	{#if me?.factors.passkeys && passkeysSupported()}
		<button class="btn wide" onclick={withPasskey} disabled={busy}>Use a passkey</button>
	{/if}
	<!-- A password alone is never a step-up: it needs the authenticator code too. -->
	{#if me?.factors.password && me.factors.totp}
		<form onsubmit={withPassword}>
			<label class="field">Password <input type="password" bind:value={password} autocomplete="current-password" required maxlength="1024" /></label>
			<label class="field">Authenticator code <input bind:value={code} inputmode="numeric" autocomplete="one-time-code" required maxlength="8" /></label>
			<button class="btn soft wide" disabled={busy}>Confirm</button>
		</form>
	{/if}
	<p class="error" role="alert">{error}</p>
</Sheet>

<style>
	.wide {
		width: 100%;
		margin-bottom: 0.75rem;
		min-height: 48px;
	}
	form {
		margin-top: 0.5rem;
		padding-top: 1rem;
		border-top: 1px solid var(--line);
	}
</style>
