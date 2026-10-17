<script lang="ts">
	// The library (SPEC §7.2): search, filter by status, sort; episodes of a show appear as
	// one tile. With `shelf`, only that genre (or `fiction` / `non-fiction`).
	import type { Snippet } from 'svelte';
	import { api } from '$lib/api/session.svelte';
	import { messageOf } from '$lib/api/http';
	import type { Page, Tile } from '$lib/api/types';
	import BookTile from './BookTile.svelte';

	let { title, shelf = null, header }: { title: string; shelf?: string | null; header?: Snippet } = $props();

	const SORTS = [
		['title', 'Title'],
		['author', 'Author'],
		['added', 'Recently added'],
		['recent', 'Recently played'],
	] as const;
	const STATUSES = [
		['', 'All'],
		['not_started', 'Not started'],
		['in_progress', 'In progress'],
		['finished', 'Finished'],
	] as const;

	let q = $state('');
	let sort = $state<string>('title');
	let status = $state<string>('');
	let items = $state<Tile[]>([]);
	let total = $state(0);
	let next = $state<string | null>(null);
	let error = $state('');
	let loading = $state(false);
	let seq = 0;

	function query(cursor: string | null) {
		const p = new URLSearchParams({ sort, limit: '60' });
		if (q.trim()) p.set('q', q.trim());
		if (status) p.set('status', status);
		if (shelf) p.set('shelf', shelf);
		if (cursor) p.set('cursor', cursor);
		return `v1/books?${p}`;
	}

	async function load(more = false) {
		const mine = ++seq;
		loading = true;
		error = '';
		try {
			const r = await api<Page<Tile>>(query(more ? next : null));
			if (mine !== seq) return;
			items = more ? [...items, ...r.items] : r.items;
			total = r.total;
			next = r.next;
		} catch (e) {
			if (mine === seq) error = messageOf(e);
		} finally {
			if (mine === seq) loading = false;
		}
	}

	// Reload as the search, sort or filter change (typing waits for a pause).
	let timer: ReturnType<typeof setTimeout> | undefined;
	$effect(() => {
		void q;
		void sort;
		void status;
		void shelf;
		clearTimeout(timer);
		timer = setTimeout(() => void load(false), 250);
		return () => clearTimeout(timer);
	});
</script>

<main class="page">
	<h1>{title}</h1>
	{@render header?.()}
	<div class="controls">
		<label class="search">
			<span class="visually-hidden">Search</span>
			<input type="search" bind:value={q} placeholder="Search titles, authors, narrators, series" autocomplete="off" maxlength="200" />
		</label>
		<label><span class="visually-hidden">Show</span>
			<select bind:value={status}>
				{#each STATUSES as [v, l] (v)}<option value={v}>{l}</option>{/each}
			</select>
		</label>
		<label><span class="visually-hidden">Sort by</span>
			<select bind:value={sort}>
				{#each SORTS as [v, l] (v)}<option value={v}>{l}</option>{/each}
			</select>
		</label>
	</div>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	<p class="muted count" aria-live="polite">{loading && !items.length ? 'Loading…' : `${total} ${total === 1 ? 'title' : 'titles'}`}</p>
	<div class="grid">
		{#each items as t (t.type + t.id)}<BookTile tile={t} />{/each}
	</div>
	{#if next}
		<button class="btn plain more" onclick={() => load(true)} disabled={loading}>Show more</button>
	{/if}
</main>

<style>
	.controls {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
		margin-bottom: 0.5rem;
	}
	.search {
		flex: 1 1 100%;
	}
	input,
	select {
		min-height: 44px;
		border: 1px solid var(--line);
		border-radius: var(--radius-sm);
		background: var(--bg-raised);
		padding: 0 0.8em;
	}
	input {
		width: 100%;
	}
	.count {
		font-size: 0.85rem;
	}
	.grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(130px, 1fr));
		gap: 1rem;
	}
	.more {
		display: flex;
		margin: 1.5rem auto 0;
	}
</style>
