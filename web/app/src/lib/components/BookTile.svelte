<script lang="ts">
	import type { Tile } from '$lib/api/types';
	import { downloads } from '$lib/offline/downloads.svelte';
	import { names, timeLeft } from '$lib/format';
	import Cover from './Cover.svelte';
	import Icon from './Icon.svelte';

	let { tile }: { tile: Tile } = $props();
	const href = $derived(tile.type === 'show' ? `#/group/${tile.id}` : `#/book/${tile.id}`);
	const progress = $derived(tile.type === 'book' ? tile.progress : null);
	const pct = $derived(tile.type === 'book' && progress && !progress.finished ? Math.min(100, (progress.position_ms / Math.max(1, tile.duration_ms)) * 100) : 0);
</script>

<a class="tile" {href}>
	<div class="art">
		<Cover id={tile.id} src={tile.cover} title={tile.title} />
		{#if tile.type === 'book' && downloads.byId(tile.id)?.state === 'complete'}
			<span class="badge" title="Downloaded"><Icon name="downloaded" size={18} label="Downloaded" /></span>
		{/if}
		{#if pct > 0}
			<span class="bar" style:width="{pct}%"></span>
		{/if}
	</div>
	<span class="title">{tile.title}</span>
	{#if tile.type === 'show'}
		<span class="sub muted">{tile.episodes} episodes</span>
	{:else if progress?.finished}
		<span class="sub muted"><Icon name="check" size={14} /> Finished</span>
	{:else if pct > 0}
		<span class="sub muted">{timeLeft(tile.duration_ms, progress?.position_ms ?? 0)}</span>
	{:else}
		<span class="sub muted">{names(tile.authors)}</span>
	{/if}
</a>

<style>
	.tile {
		display: grid;
		gap: 0.25rem;
		color: inherit;
		text-decoration: none;
		min-width: 0;
	}
	.art {
		position: relative;
		overflow: hidden;
		border-radius: var(--radius-sm);
	}
	.badge {
		position: absolute;
		right: 6px;
		top: 6px;
		display: grid;
		place-items: center;
		background: var(--accent);
		color: var(--accent-fg);
		border-radius: 50%;
		padding: 2px;
	}
	.bar {
		position: absolute;
		left: 0;
		bottom: 0;
		height: 4px;
		background: var(--accent);
	}
	.title {
		font-weight: 600;
		line-height: 1.25;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}
	.sub {
		font-size: 0.85rem;
		display: flex;
		align-items: center;
		gap: 0.25em;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
</style>
