<script lang="ts">
	import { player } from '$lib/player/player.svelte';
	import Cover from './Cover.svelte';
	import Icon from './Icon.svelte';

	const b = $derived(player.book);
	const pct = $derived(b ? Math.min(100, (player.position / Math.max(1, player.duration)) * 100) : 0);
	const line = $derived(b && player.chapter >= 0 ? b.chapters[player.chapter]?.title : b ? (b.authors ?? []).map((p) => p.name).join(', ') : '');
</script>

{#if b}
	<div class="mini" role="region" aria-label="Now playing">
		<span class="bar" style:width="{pct}%"></span>
		<button class="open" onclick={() => (player.expanded = true)} aria-label="Open the player: {b.title}">
			<span class="art"><Cover id={b.id} src={b.cover} title={b.title} size={128} /></span>
			<span class="text">
				<span class="title">{b.title}</span>
				<span class="muted line">{player.notice ?? line}</span>
			</span>
		</button>
		<button class="icon-btn play" onclick={() => player.toggle()} aria-label={player.playing ? 'Pause' : 'Play'} disabled={player.preparing}>
			<Icon name={player.playing ? 'pause' : 'play'} size={28} />
		</button>
	</div>
{/if}

<style>
	.mini {
		position: fixed;
		left: 0;
		right: 0;
		bottom: calc(var(--tabs-h) + var(--safe-b));
		height: var(--mini-h);
		display: flex;
		align-items: center;
		gap: 0.5rem;
		padding: 0 0.5rem 0 0;
		background: var(--bg-raised);
		border-top: 1px solid var(--line);
		z-index: 20;
	}
	.bar {
		position: absolute;
		top: -1px;
		left: 0;
		height: 2px;
		background: var(--accent);
	}
	.open {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 0.75rem;
		height: 100%;
		padding: 0 0 0 0.6rem;
		border: 0;
		background: transparent;
		text-align: left;
	}
	.art {
		width: 48px;
		flex: none;
	}
	.text {
		display: grid;
		min-width: 0;
	}
	.title,
	.line {
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.title {
		font-weight: 600;
	}
	.line {
		font-size: 0.85rem;
	}
	.play {
		width: 52px;
		height: 52px;
	}
</style>
