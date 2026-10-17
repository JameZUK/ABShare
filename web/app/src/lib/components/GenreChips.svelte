<script lang="ts">
	// A scrolling row of genre chips under the library title: the umbrella views, the
	// biggest shelves, and the full Genres screen.
	import { loadGenres, type Genre } from '$lib/api/genres';
	import { session } from '$lib/api/session.svelte';

	let genres = $state<Genre[]>([]);
	$effect(() => {
		if (session.status !== 'signed-in') return;
		loadGenres()
			.then((r) => (genres = [...r.genres].sort((a, b) => b.count - a.count).slice(0, 8)))
			.catch(() => {});
	});
</script>

{#if genres.length}
	<nav class="chips" aria-label="Genres">
		<a href="#/genres" class="chip all">All genres</a>
		<a href="#/genre/fiction" class="chip">Fiction</a>
		<a href="#/genre/non-fiction" class="chip">Non-fiction</a>
		{#each genres as g (g.id)}<a href="#/genre/{g.id}" class="chip">{g.name}</a>{/each}
	</nav>
{/if}

<style>
	.chips {
		display: flex;
		gap: 0.5rem;
		overflow-x: auto;
		padding-bottom: 0.5rem;
		margin-bottom: 0.5rem;
	}
	.chip {
		flex: none;
		display: inline-flex;
		align-items: center;
		min-height: 36px;
		padding: 0 0.9rem;
		border-radius: 999px;
		border: 1px solid var(--line);
		color: inherit;
		text-decoration: none;
		font-size: 0.9rem;
		white-space: nowrap;
	}
	.chip.all {
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
		border-color: transparent;
		font-weight: 600;
	}
</style>
