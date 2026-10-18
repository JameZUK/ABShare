<script lang="ts">
	import type { Tile } from '$lib/api/types';
	import { downloads } from '$lib/offline/downloads.svelte';
	import { names, timeLeft } from '$lib/format';
	import Cover from './Cover.svelte';
	import Icon from './Icon.svelte';

	let { tile, sub = '' }: { tile: Tile; sub?: string } = $props();
	const href = $derived(tile.type === 'show' ? `#/group/${tile.id}` : `#/book/${tile.id}`);
	const progress = $derived(tile.type === 'book' ? tile.progress : null);
	const pct = $derived(tile.type === 'book' && progress && !progress.finished ? Math.min(100, (progress.position_ms / Math.max(1, tile.duration_ms)) * 100) : 0);
</script>

<a class="tile" {href}>
	<span class="art">
		<Cover id={tile.id} src={tile.cover} title={tile.title} book />
		{#if tile.type === 'book' && downloads.byId(tile.id)?.state === 'complete'}
			<span class="badge" title="Downloaded"><Icon name="downloaded" size={16} label="Downloaded" /></span>
		{/if}
	</span>
	{#if pct > 0}
		<span class="progress" aria-hidden="true"><span style:width="{pct}%"></span></span>
	{/if}
	<span class="title">{tile.title}</span>
	{#if sub}
		<span class="sub muted">{sub}</span>
	{:else if tile.type === 'show'}
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
		align-content: start;
		gap: 0.2rem;
		color: inherit;
		text-decoration: none;
		min-width: 0;
	}
	.art {
		position: relative;
		display: block;
		margin-bottom: 0.55rem;
		transition: transform 0.2s var(--ease);
	}
	.tile:hover .art {
		transform: translateY(-2px);
	}
	.badge {
		position: absolute;
		right: 6px;
		top: 6px;
		display: grid;
		place-items: center;
		width: 26px;
		height: 26px;
		background: var(--fg);
		color: var(--bg);
		border-radius: 50%;
	}
	.progress {
		display: block;
		height: 2px;
		background: var(--line);
		margin: -0.15rem 0 0.35rem;
	}
	.progress span {
		display: block;
		height: 100%;
		background: var(--fg);
	}
	.title {
		font-family: var(--font-display);
		font-weight: 600;
		font-size: 1rem;
		line-height: 1.2;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}
	.sub {
		font-size: 0.82rem;
		display: flex;
		align-items: center;
		gap: 0.25em;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
</style>
