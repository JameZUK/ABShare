<script lang="ts">
	import type { Tile } from '$lib/api/types';
	import BookTile from './BookTile.svelte';

	let {
		title,
		tiles,
		subs = [],
		more = '',
	}: { title: string; tiles: Tile[]; subs?: string[]; more?: string } = $props();
</script>

{#if tiles.length}
	<section>
		<header>
			<h2>{title}</h2>
			{#if more}<a href={more}>See all</a>{/if}
		</header>
		<div class="shelf">
			{#each tiles as t, i (t.type + t.id)}<BookTile tile={t} sub={subs[i] ?? ''} />{/each}
		</div>
	</section>
{/if}

<style>
	section {
		margin-bottom: 2rem;
	}
	header {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		margin-bottom: 0.9rem;
	}
	h2 {
		margin: 0;
	}
	header a {
		font-size: 0.85rem;
		font-weight: 600;
		text-decoration: none;
	}
	/* A row that scrolls sideways on phones and lays out as a grid when there's room. */
	.shelf {
		display: grid;
		grid-auto-flow: column;
		grid-auto-columns: 132px;
		gap: 1.1rem;
		overflow-x: auto;
		scroll-snap-type: x proximity;
		scrollbar-width: none;
		margin: 0 calc(-1 * var(--gutter));
		padding: 0.25rem var(--gutter) 1rem;
		scroll-padding-inline: var(--gutter);
	}
	.shelf::-webkit-scrollbar {
		display: none;
	}
	.shelf > :global(*) {
		scroll-snap-align: start;
	}
	@media (min-width: 1024px) {
		.shelf {
			grid-auto-columns: 156px;
			gap: 1.5rem;
		}
	}
</style>
