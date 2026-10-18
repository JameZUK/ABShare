<script lang="ts">
	// Genre chips: the umbrella views, the biggest shelves, and the full Genres screen. A
	// sideways-scrolling row by default; `wrap` lays them out over several lines.
	import { loadGenres, type Genre } from '$lib/api/genres';
	import { session } from '$lib/api/session.svelte';

	let { wrap = false }: { wrap?: boolean } = $props();
	let genres = $state<Genre[]>([]);
	$effect(() => {
		if (session.status !== 'signed-in') return;
		loadGenres()
			.then((r) => (genres = [...r.genres].sort((a, b) => b.count - a.count).slice(0, 8)))
			.catch(() => {});
	});
</script>

{#if genres.length}
	<nav class="chips" class:scroll={!wrap} aria-label="Genres">
		<a href="#/genre/fiction" class="chip">Fiction</a>
		<a href="#/genre/non-fiction" class="chip">Non-fiction</a>
		{#each genres as g (g.id)}<a href="#/genre/{g.id}" class="chip">{g.name}</a>{/each}
		<a href="#/genres" class="chip all">All genres</a>
	</nav>
{/if}

<style>
	.scroll {
		flex-wrap: nowrap;
		overflow-x: auto;
		scrollbar-width: none;
		margin: 0 calc(-1 * var(--gutter)) 1rem;
		padding: 0 var(--gutter) 0.25rem;
	}
	.scroll::-webkit-scrollbar {
		display: none;
	}
	.chip {
		flex: none;
	}
	.all {
		color: var(--accent);
		border-color: transparent;
		font-weight: 600;
	}
</style>
