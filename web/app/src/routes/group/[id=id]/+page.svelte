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
	<button class="icon-btn" onclick={() => history.back()} aria-label="Back"><Icon name="back" /></button>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#if group}
		<h1>{group.name}{group.qualifier ? ` (${group.qualifier})` : ''}</h1>
		<p class="muted">{main.length} {group.kind === 'show' ? 'episodes' : 'books'}</p>
		{#if nextBook}
			<p><a class="btn" href="#/book/{nextBook.id}">Next: {nextBook.title}</a></p>
		{/if}
		<div class="grid">
			{#each main as b (b.id)}<BookTile tile={b} />{/each}
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
	.grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(130px, 1fr));
		gap: 1rem;
		margin-bottom: 1.5rem;
	}
</style>
