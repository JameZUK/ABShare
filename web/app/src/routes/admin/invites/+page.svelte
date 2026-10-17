<script lang="ts">
	import { adminApi, librariesLabel, type Invite } from '$lib/api/admin';
	import { ApiError, messageOf } from '$lib/api/http';
	import { session } from '$lib/api/session.svelte';
	import { date } from '$lib/format';
	import { withStepUp } from '$lib/stepup.svelte';
	import AdminShell from '$lib/components/AdminShell.svelte';
	import CopyLink from '$lib/components/CopyLink.svelte';
	import Icon from '$lib/components/Icon.svelte';

	let invites = $state<Invite[]>([]);
	let error = $state('');
	let link = $state<string | null>(null);
	let busy = $state(false);
	let name = $state('');
	let role = $state<'listener' | 'admin'>('listener');
	let confirmAdmin = $state(false);
	let libs = $state('');
	let offline = $state(true);
	let saveFiles = $state(false);
	let hours = $state(72);

	async function reload() {
		try {
			invites = await adminApi.invites();
		} catch (e) {
			error = messageOf(e);
		}
	}
	$effect(() => {
		if (session.status === 'signed-in' && session.user?.role === 'admin') void reload();
	});

	async function create(e: Event) {
		e.preventDefault();
		busy = true;
		error = '';
		link = null;
		try {
			const names = libs.split(',').map((s) => s.trim()).filter(Boolean);
			const r = await withStepUp(() =>
				adminApi.createInvite({
					display_name: name.trim() || undefined,
					role,
					confirm_admin: role === 'admin' ? confirmAdmin : undefined,
					libraries: names.length ? names : '*',
					offline,
					save_files: saveFiles,
					expires_hours: hours,
				}),
			);
			link = r.link;
			name = '';
			confirmAdmin = false;
			await reload();
		} catch (err) {
			error = err instanceof ApiError && err.code === 'confirm_admin' ? 'Tick the box to confirm this person should be an admin.' : messageOf(err);
		} finally {
			busy = false;
		}
	}

	async function revoke(i: Invite) {
		error = '';
		try {
			await withStepUp(() => adminApi.revokeInvite(i.id));
			await reload();
		} catch (e) {
			error = messageOf(e);
		}
	}
</script>

<AdminShell title="Invites">
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#if link}<CopyLink {link} note="Send this link privately (a message, not a public post). It works once, for {hours} hours." />{/if}
	<form class="card" onsubmit={create}>
		<h2>Invite someone</h2>
		<label class="field">Their name (optional) <input bind:value={name} maxlength="64" placeholder="Shown when they open the link" /></label>
		<label class="field">Role
			<select bind:value={role}>
				<option value="listener">Listener</option>
				<option value="admin">Admin</option>
			</select>
		</label>
		{#if role === 'admin'}
			<label class="check"><input type="checkbox" bind:checked={confirmAdmin} /> Yes, this person can manage everyone's accounts</label>
		{/if}
		<label class="field">Libraries (comma-separated; empty means all) <input bind:value={libs} placeholder="All libraries" /></label>
		<label class="check"><input type="checkbox" bind:checked={offline} /> Can download for offline listening</label>
		<label class="check"><input type="checkbox" bind:checked={saveFiles} /> Can save books as files</label>
		<label class="field">Link lasts
			<select bind:value={hours}>
				<option value={24}>1 day</option>
				<option value={72}>3 days</option>
				<option value={168}>7 days</option>
			</select>
		</label>
		<button class="btn" disabled={busy}>Create invite link</button>
	</form>

	<h2>Waiting to be used</h2>
	{#each invites as i (i.id)}
		<div class="row item">
			<span class="spacer">
				<strong>{i.purpose === 'reset' ? `Sign-in reset for @${i.for_user}` : (i.display_name ?? 'Unnamed invite')}</strong>
				{#if i.role === 'admin'}<span class="muted"> · admin</span>{/if}<br />
				<span class="muted small">{librariesLabel(i.libraries)} · expires {date(i.expires_at)}</span>
			</span>
			<button class="icon-btn" onclick={() => revoke(i)} aria-label="Cancel this invite"><Icon name="trash" /></button>
		</div>
	{:else}
		<p class="muted">None.</p>
	{/each}
</AdminShell>

<style>
	form {
		margin-bottom: 1.5rem;
		max-width: 520px;
	}
	.check {
		display: flex;
		gap: 0.6rem;
		align-items: center;
		min-height: 44px;
	}
	.item {
		min-height: 60px;
		border-bottom: 1px solid var(--line);
	}
	.small {
		font-size: 0.85rem;
	}
</style>
