<script lang="ts">
	import { messageOf } from '$lib/api/http';
	import { api, session } from '$lib/api/session.svelte';
	import type { Device } from '$lib/api/types';
	import { date } from '$lib/format';
	import Icon from '$lib/components/Icon.svelte';

	let devices = $state<Device[]>([]);
	let error = $state('');

	async function reload() {
		try {
			devices = (await api<{ devices: Device[] }>('v1/me/devices')).devices;
		} catch (e) {
			error = messageOf(e);
		}
	}
	$effect(() => {
		if (session.status === 'signed-in') void reload();
	});

	async function revoke(d: Device) {
		error = '';
		try {
			await api(`v1/me/devices/${d.id}`, { method: 'DELETE' });
			await reload();
		} catch (e) {
			error = messageOf(e);
		}
	}
</script>

<main class="page narrow">
	<button class="icon-btn" onclick={() => history.back()} aria-label="Back"><Icon name="back" /></button>
	<h1>Devices</h1>
	<p class="muted">Where you’re signed in. Sign out any you don’t recognise.</p>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#each devices as d (d.id)}
		<div class="item row">
			<Icon name="device" />
			<span class="spacer">
				<strong>{d.name}</strong>{#if d.this_device}<span class="tag">This device</span>{/if}<br />
				<span class="muted small">Signed in {date(d.created_at)}{d.last_seen ? ` · last seen ${date(d.last_seen)}` : ''}</span>
			</span>
			{#if !d.this_device}
				<button class="btn plain" onclick={() => revoke(d)}>Sign out</button>
			{/if}
		</div>
	{/each}
</main>

<style>
	.item {
		min-height: 64px;
		border-bottom: 1px solid var(--line);
	}
	.small {
		font-size: 0.85rem;
	}
	.tag {
		margin-left: 0.5rem;
		padding: 0.1rem 0.5rem;
		border-radius: 999px;
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
		font-size: 0.75rem;
	}
</style>
