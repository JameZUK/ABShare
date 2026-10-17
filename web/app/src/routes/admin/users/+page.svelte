<script lang="ts">
	// People: who can sign in, what they can do, and the controls for when something's wrong.
	import { adminApi, librariesLabel, type AdminUser } from '$lib/api/admin';
	import { messageOf } from '$lib/api/http';
	import { session } from '$lib/api/session.svelte';
	import { date } from '$lib/format';
	import { withStepUp } from '$lib/stepup.svelte';
	import AdminShell from '$lib/components/AdminShell.svelte';
	import CopyLink from '$lib/components/CopyLink.svelte';
	import Sheet from '$lib/components/Sheet.svelte';

	let users = $state<AdminUser[]>([]);
	let error = $state('');
	let editing = $state<AdminUser | null>(null);
	let open = $state(false);
	let resetLink = $state<string | null>(null);
	let note = $state('');
	let busy = $state(false);
	let libs = $state('');

	async function reload() {
		try {
			users = await adminApi.users();
		} catch (e) {
			error = messageOf(e);
		}
	}
	$effect(() => {
		if (session.status === 'signed-in' && session.user?.role === 'admin') void reload();
	});

	function edit(u: AdminUser) {
		editing = { ...u, permissions: { ...u.permissions } };
		libs = u.permissions.libraries === '*' ? '' : u.permissions.libraries.join(', ');
		resetLink = null;
		note = '';
		open = true;
	}

	async function run(fn: () => Promise<void>) {
		busy = true;
		error = '';
		try {
			await withStepUp(fn);
			await reload();
		} catch (e) {
			error = messageOf(e);
		} finally {
			busy = false;
		}
	}

	const save = () =>
		run(async () => {
			const u = editing!;
			const names = libs.split(',').map((s) => s.trim()).filter(Boolean);
			await adminApi.editUser(u.id, {
				display_name: u.display_name,
				role: u.role,
				libraries: names.length ? names : '*',
				offline: u.permissions.offline,
				save_files: u.permissions.save_files,
			});
			note = 'Saved.';
		});
	const setDisabled = (d: boolean) => run(async () => {
		editing = await adminApi.editUser(editing!.id, { disabled: d });
		note = d ? 'Disabled and signed out everywhere.' : 'Enabled again.';
	});
	const reset = () => run(async () => {
		resetLink = (await adminApi.resetUser(editing!.id)).link;
	});
	const signOut = () => run(async () => {
		const n = (await adminApi.signOutUser(editing!.id)).signed_out;
		note = `Signed out of ${n} ${n === 1 ? 'device' : 'devices'}.`;
	});
</script>

<AdminShell title="Users">
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	<ul class="list">
		{#each users as u (u.id)}
			<li>
				<button class="item" onclick={() => edit(u)}>
					<span class="main">
						<strong>{u.display_name}</strong> <span class="muted">@{u.username}</span>
						{#if u.role === 'admin'}<span class="tag">Admin</span>{/if}
						{#if u.disabled}<span class="tag off">Disabled</span>{/if}
					</span>
					<span class="muted small">
						{u.factors.passkeys} passkey{u.factors.passkeys === 1 ? '' : 's'}{u.factors.totp ? ' · authenticator' : ''} ·
						{u.devices} device{u.devices === 1 ? '' : 's'} · last seen {date(u.last_seen)} · {librariesLabel(u.permissions.libraries)}
					</span>
				</button>
			</li>
		{/each}
	</ul>
</AdminShell>

<Sheet bind:open title={editing ? editing.display_name : 'User'}>
	{#if editing}
		{#if note}<p role="status">{note}</p>{/if}
		{#if resetLink}<CopyLink link={resetLink} note="Send this link privately. When it's used, their old passkeys, password and devices stop working. It lasts 72 hours." />{/if}
		<label class="field">Display name <input bind:value={editing.display_name} maxlength="64" /></label>
		<label class="field">Role
			<select bind:value={editing.role}>
				<option value="listener">Listener</option>
				<option value="admin">Admin</option>
			</select>
		</label>
		<label class="field">Libraries (comma-separated; empty means all) <input bind:value={libs} placeholder="All libraries" /></label>
		<label class="check"><input type="checkbox" bind:checked={editing.permissions.offline} /> Can download for offline listening</label>
		<label class="check"><input type="checkbox" bind:checked={editing.permissions.save_files} /> Can save books as files</label>
		<div class="actions">
			<button class="btn" onclick={save} disabled={busy}>Save</button>
			<button class="btn plain" onclick={reset} disabled={busy}>Reset sign-in</button>
			<button class="btn plain" onclick={signOut} disabled={busy}>Sign out everywhere</button>
			{#if editing.disabled}
				<button class="btn plain" onclick={() => setDisabled(false)} disabled={busy}>Enable</button>
			{:else}
				<button class="btn plain danger" onclick={() => setDisabled(true)} disabled={busy}>Disable</button>
			{/if}
		</div>
	{/if}
</Sheet>

<style>
	.list {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.item {
		display: grid;
		gap: 0.2rem;
		width: 100%;
		min-height: 60px;
		padding: 0.6rem 0.25rem;
		border: 0;
		border-bottom: 1px solid var(--line);
		background: transparent;
		text-align: left;
	}
	.small {
		font-size: 0.85rem;
	}
	.tag {
		margin-left: 0.4rem;
		padding: 0.05rem 0.5rem;
		border-radius: 999px;
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
		font-size: 0.75rem;
	}
	.tag.off {
		background: var(--bg-sunken);
		color: var(--danger);
	}
	.check {
		display: flex;
		gap: 0.6rem;
		align-items: center;
		min-height: 44px;
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
		margin-top: 1rem;
	}
	.danger {
		color: var(--danger);
	}
</style>
