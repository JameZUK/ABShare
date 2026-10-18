<script lang="ts">
	import { player } from '$lib/player/player.svelte';
	import Cover from './Cover.svelte';
	import Icon from './Icon.svelte';

	const b = $derived(player.book);
	const pct = $derived(b ? Math.min(100, (player.position / Math.max(1, player.duration)) * 100) : 0);
	const line = $derived(b && player.chapter >= 0 ? b.chapters[player.chapter]?.title : b ? (b.authors ?? []).map((p) => p.name).join(', ') : '');
</script>

{#if b && !player.expanded}
	<div class="mini" role="region" aria-label="Now playing">
		<button class="open" onclick={() => (player.expanded = true)} aria-label="Open the player: {b.title}">
			<span class="art"><Cover id={b.id} src={b.cover} title={b.title} size={128} /></span>
			<span class="text">
				<span class="title serif">{b.title}</span>
				<span class="line">{player.notice ?? line}</span>
			</span>
		</button>
		<button class="icon-btn play" onclick={() => player.toggle()} aria-label={player.playing ? 'Pause' : 'Play'} disabled={player.preparing}>
			<Icon name={player.playing ? 'pause' : 'play'} size={24} />
		</button>
		<span class="bar" aria-hidden="true"><span style:width="{pct}%"></span></span>
	</div>
{/if}

<style>
	/* An ink card floating above the tabs (inverted: cream on ink in light, ink on cream in
	   dark), so what's playing is always easy to find. */
	.mini {
		position: fixed;
		left: calc(var(--side-w) + 0.5rem);
		right: 0.5rem;
		bottom: calc(var(--tabs-h) + var(--safe-b) + 0.5rem);
		height: var(--mini-h);
		display: flex;
		align-items: center;
		gap: 0.25rem;
		padding: 0 0.4rem 0 0;
		background: var(--fg);
		color: var(--bg);
		border-radius: var(--radius);
		box-shadow: var(--shadow);
		overflow: hidden;
		z-index: 20;
		animation: up 0.3s var(--ease);
	}
	@keyframes up {
		from {
			transform: translateY(12px);
			opacity: 0;
		}
	}
	@media (min-width: 1024px) {
		.mini {
			left: calc(var(--side-w) + 1.5rem);
			right: 1.5rem;
			bottom: 1rem;
			max-width: 760px;
			margin: 0 auto;
		}
	}
	.open {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 0.8rem;
		height: 100%;
		padding: 0 0 0 0.55rem;
		border: 0;
		background: transparent;
		color: inherit;
		text-align: left;
	}
	.art {
		width: 48px;
		flex: none;
	}
	.art :global(.frame) {
		border-radius: 6px;
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
		font-size: 1rem;
	}
	.line {
		font-size: 0.82rem;
		opacity: 0.75;
	}
	.play {
		width: 48px;
		height: 48px;
		color: inherit;
	}
	.play:hover:not(:disabled) {
		background: rgb(128 128 128 / 20%);
	}
	.bar {
		position: absolute;
		left: 0;
		right: 0;
		bottom: 0;
		height: 3px;
		background: rgb(128 128 128 / 30%);
	}
	.bar span {
		display: block;
		height: 100%;
		background: var(--accent);
	}
</style>
