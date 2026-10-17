<script lang="ts">
	import type { Tile } from '$lib/api/types';
	import BookTile from './BookTile.svelte';

	let { title, tiles, wide = false }: { title: string; tiles: Tile[]; wide?: boolean } = $props();
</script>

{#if tiles.length}
	<section>
		<h2>{title}</h2>
		<div class="shelf" class:wide>
			{#each tiles as t (t.type + t.id)}<BookTile tile={t} />{/each}
		</div>
	</section>
{/if}

<style>
	section {
		margin-bottom: 1.5rem;
	}
	.shelf {
		display: grid;
		grid-auto-flow: column;
		grid-auto-columns: minmax(120px, 140px);
		gap: 0.9rem;
		overflow-x: auto;
		scroll-snap-type: x proximity;
		padding-bottom: 0.25rem;
	}
	.shelf.wide {
		grid-auto-columns: minmax(150px, 180px);
	}
	.shelf > :global(*) {
		scroll-snap-align: start;
	}
</style>
