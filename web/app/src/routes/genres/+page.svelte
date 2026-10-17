<script lang="ts">
	// Browse by genre: fiction and non-fiction shelves, with how many books are on each.
	import { messageOf } from '$lib/api/http';
	import { loadGenres, type Genre } from '$lib/api/genres';
	import { session } from '$lib/api/session.svelte';
	import Icon from '$lib/components/Icon.svelte';

	let genres = $state<Genre[]>([]);
	let unshelved = $state(0);
	let error = $state('');
	$effect(() => {
		if (session.status !== 'signed-in') return;
		loadGenres()
			.then((r) => {
				genres = r.genres;
				unshelved = r.unshelved;
			})
			.catch((e) => (error = messageOf(e)));
	});
	const groups = $derived([
		{ kind: 'fiction', title: 'Fiction', items: genres.filter((g) => g.kind === 'fiction') },
		{ kind: 'non-fiction', title: 'Non-fiction', items: genres.filter((g) => g.kind === 'non-fiction') },
		{ kind: 'other', title: 'Other', items: genres.filter((g) => g.kind === 'other') },
	]);
</script>

<main class="page">
	<button class="icon-btn" onclick={() => history.back()} aria-label="Back"><Icon name="back" /></button>
	<h1>Genres</h1>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#each groups as grp (grp.kind)}
		{#if grp.items.length}
			<section>
				<div class="row">
					<h2>{grp.title}</h2>
					<span class="spacer"></span>
					{#if grp.kind !== 'other'}<a href="#/genre/{grp.kind}">See all</a>{/if}
				</div>
				<div class="grid">
					{#each grp.items as g (g.id)}
						<a class="tile" href="#/genre/{g.id}">
							<span class="name">{g.name}</span>
							<span class="muted">{g.count} {g.count === 1 ? 'book' : 'books'}</span>
						</a>
					{/each}
				</div>
			</section>
		{/if}
	{/each}
	{#if unshelved}
		<p class="muted small">{unshelved} {unshelved === 1 ? 'book has' : 'books have'} no genre in its details. Find them in the <a href="#/library">library</a>.</p>
	{/if}
</main>

<style>
	section {
		margin-bottom: 1.5rem;
	}
	h2 {
		margin: 0;
	}
	.row {
		margin-bottom: 0.6rem;
	}
	.grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
		gap: 0.75rem;
	}
	.tile {
		display: grid;
		gap: 0.2rem;
		align-content: center;
		min-height: 76px;
		padding: 0.75rem 1rem;
		border-radius: var(--radius-sm);
		background: var(--bg-raised);
		box-shadow: var(--shadow);
		color: inherit;
		text-decoration: none;
	}
	.name {
		font-weight: 600;
	}
	.small {
		font-size: 0.85rem;
	}
</style>
