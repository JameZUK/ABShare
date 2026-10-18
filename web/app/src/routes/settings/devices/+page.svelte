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
	<button class="icon-btn back" onclick={() => history.back()} aria-label="Back"><Icon name="back" /></button>
	<h1>Devices</h1>
	<p class="muted">Where you’re signed in. Sign out any you don’t recognise.</p>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	<ul class="rules list">
		{#each devices as d (d.id)}
			<li class="item">
				<span class="ic"><Icon name="device" size={20} /></span>
				<span class="text">
					<span class="name"><strong>{d.name}</strong>{#if d.this_device}<span class="tag">This device</span>{/if}</span>
					<span class="muted small">Signed in {date(d.created_at)}{d.last_seen ? ` · last seen ${date(d.last_seen)}` : ''}</span>
				</span>
				{#if !d.this_device}
					<button class="btn soft" onclick={() => revoke(d)}>Sign out</button>
				{/if}
			</li>
		{/each}
	</ul>
</main>

<style>
	.back {
		margin-left: -0.6rem;
	}
	.list {
		list-style: none;
		margin: 1.25rem 0 0;
		padding: 0;
		border-top: 1px solid var(--line);
		border-bottom: 1px solid var(--line);
	}
	.item {
		display: flex;
		align-items: center;
		gap: 1rem;
		min-height: 72px;
		padding: 0.75rem 0;
	}
	.ic {
		display: grid;
		place-items: center;
		width: 40px;
		height: 40px;
		flex: none;
		border: 1px solid var(--line);
		border-radius: 50%;
	}
	.text {
		flex: 1;
		display: grid;
		gap: 0.15rem;
		min-width: 0;
	}
	.name {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 0.5rem;
	}
	.small {
		font-size: 0.85rem;
	}
	.tag {
		padding: 0.05rem 0.55rem;
		border-radius: 999px;
		border: 1px solid var(--accent);
		color: var(--accent);
		font-size: 0.72rem;
		font-weight: 600;
		letter-spacing: 0.04em;
	}
</style>
