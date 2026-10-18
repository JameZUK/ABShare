<script lang="ts">
	// A series or a show: its books or episodes in order, and which is next.
	import { page } from '$app/state';
	import { api } from '$lib/api/session.svelte';
	import { messageOf } from '$lib/api/http';
	import type { GroupDetail } from '$lib/api/types';
	import BookTile from '$lib/components/BookTile.svelte';
	import Icon from '$lib/components/Icon.svelte';

	const id = $derived(page.params.id ?? '');
	let group = $state<GroupDetail | null>(null);
	let error = $state('');

	$effect(() => {
		const gid = id;
		group = null;
		api<GroupDetail>(`v1/groups/${gid}`)
			.then((g) => (group = g))
			.catch((e) => (error = messageOf(e)));
	});

	const main = $derived(group?.items.filter((b) => !b.extra) ?? []);
	const extras = $derived(group?.items.filter((b) => b.extra) ?? []);
	const nextBook = $derived(group?.items.find((b) => b.id === group?.next));
</script>

<main class="page">
	<button class="icon-btn back" onclick={() => history.back()} aria-label="Back"><Icon name="back" /></button>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#if group}
		<span class="eyebrow accent">{group.kind === 'show' ? 'Show' : 'Series'}</span>
		<h1>{group.name}{group.qualifier ? ` (${group.qualifier})` : ''}</h1>
		<div class="intro">
			<p class="muted">{main.length} {group.kind === 'show' ? 'episodes' : 'books'}</p>
			{#if nextBook}
				<a class="btn" href="#/book/{nextBook.id}"><Icon name="play" size={16} /> Next: {nextBook.title}</a>
			{/if}
		</div>
		<div class="grid">
			{#each main as b (b.id)}<BookTile tile={b} sub={group.kind === 'series' && b.series?.position ? `Book ${b.series.position}` : ''} />{/each}
		</div>
		{#if extras.length}
			<h2>Extras</h2>
			<div class="grid">
				{#each extras as b (b.id)}<BookTile tile={b} />{/each}
			</div>
		{/if}
	{/if}
</main>

<style>
	.back {
		margin: 0 0 0.5rem -0.6rem;
	}
	h1 {
		margin-bottom: 0.5rem;
	}
	.intro {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: 0.75rem;
		padding-bottom: 1.25rem;
		margin-bottom: 1.5rem;
		border-bottom: 1px solid var(--line);
	}
	.intro p {
		margin: 0;
	}
	h2 {
		padding-top: 1.5rem;
		border-top: 1px solid var(--line);
	}
	.grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
		gap: 1.6rem 1.1rem;
		margin-bottom: 2rem;
	}
	@media (min-width: 1024px) {
		.grid {
			grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
			gap: 2rem 1.5rem;
		}
	}
</style>
