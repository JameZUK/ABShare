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
	<button class="icon-btn back" onclick={() => history.back()} aria-label="Back"><Icon name="back" /></button>
	<span class="eyebrow accent">Browse</span>
	<h1>Genres</h1>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	<div class="cols">
		{#each groups as grp (grp.kind)}
			{#if grp.items.length}
				<section>
					<header>
						<h2>{grp.title}</h2>
						{#if grp.kind !== 'other'}<a href="#/genre/{grp.kind}">See all</a>{/if}
					</header>
					<ul>
						{#each grp.items as g (g.id)}
							<li>
								<a href="#/genre/{g.id}">
									<span class="name serif">{g.name}</span>
									<span class="dots" aria-hidden="true"></span>
									<span class="count muted">{g.count}<span class="visually-hidden"> {g.count === 1 ? 'book' : 'books'}</span></span>
								</a>
							</li>
						{/each}
					</ul>
				</section>
			{/if}
		{/each}
	</div>
	{#if unshelved}
		<p class="muted small">{unshelved} {unshelved === 1 ? 'book has' : 'books have'} no genre in its details. Find them in the <a href="#/library">library</a>.</p>
	{/if}
</main>

<style>
	.back {
		margin: 0 0 0.5rem -0.6rem;
	}
	h1 {
		margin-bottom: 1.5rem;
	}
	/* Laid out like a book's contents page: name, leader dots, count. */
	.cols {
		display: grid;
		gap: 0 3rem;
	}
	@media (min-width: 900px) {
		.cols {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}
	}
	section {
		margin-bottom: 2rem;
	}
	header {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		padding-bottom: 0.5rem;
		border-bottom: 1px solid var(--fg);
	}
	header a {
		font-size: 0.85rem;
		font-weight: 600;
		text-decoration: none;
	}
	h2 {
		margin: 0;
	}
	ul {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	li + li {
		border-top: 1px solid var(--line);
	}
	li a {
		display: flex;
		align-items: baseline;
		gap: 0.5rem;
		min-height: 50px;
		padding: 0.8rem 0;
		color: inherit;
		text-decoration: none;
	}
	li a:hover .name {
		color: var(--accent);
	}
	.name {
		font-size: 1.1rem;
		font-weight: 500;
	}
	.dots {
		flex: 1;
		border-bottom: 1px dotted var(--line);
		transform: translateY(-0.3em);
	}
	.count {
		font-variant-numeric: tabular-nums;
		font-size: 0.9rem;
	}
	.small {
		font-size: 0.85rem;
	}
</style>
