<script lang="ts">
	// The library (SPEC §7.2): search, filter by status, sort; episodes of a show appear as
	// one tile. With `shelf`, only that genre (or `fiction` / `non-fiction`).
	import type { Snippet } from 'svelte';
	import { api } from '$lib/api/session.svelte';
	import { messageOf } from '$lib/api/http';
	import type { Page, Tile } from '$lib/api/types';
	import BookTile from './BookTile.svelte';
	import Icon from './Icon.svelte';

	let {
		title,
		shelf = null,
		author = null,
		narrator = null,
		eyebrow = '',
		back = false,
		header,
	}: { title: string; shelf?: string | null; author?: string | null; narrator?: string | null; eyebrow?: string; back?: boolean; header?: Snippet } = $props();

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
		if (author) p.set('author', author);
		if (narrator) p.set('narrator', narrator);
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
		void author;
		void narrator;
		clearTimeout(timer);
		timer = setTimeout(() => void load(false), 250);
		return () => clearTimeout(timer);
	});
</script>

<main class="page">
	{#if back}<button class="icon-btn back" onclick={() => history.back()} aria-label="Back"><Icon name="back" /></button>{/if}
	{#if eyebrow}<span class="eyebrow accent">{eyebrow}</span>{/if}
	<h1>{title}</h1>
	{@render header?.()}
	<label class="search">
		<span class="visually-hidden">Search</span>
		<Icon name="search" size={20} />
		<input type="search" bind:value={q} placeholder="Search titles, authors, narrators, series" autocomplete="off" maxlength="200" />
	</label>
	<div class="filters"><div class="segmented" role="radiogroup" aria-label="Show">
		{#each STATUSES as [v, l] (v)}<label><input type="radio" name="status" value={v} bind:group={status} />{l}</label>{/each}
	</div></div>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	<div class="bar">
		<p class="muted count" aria-live="polite">{loading && !items.length ? 'Loading…' : `${total} ${total === 1 ? 'title' : 'titles'}`}</p>
		<label class="sort"><span class="muted">Sort</span>
			<select class="select" bind:value={sort}>
				{#each SORTS as [v, l] (v)}<option value={v}>{l}</option>{/each}
			</select>
		</label>
	</div>
	<div class="grid">
		{#each items as t (t.type + t.id)}<BookTile tile={t} />{/each}
	</div>
	{#if next}
		<button class="btn soft more" onclick={() => load(true)} disabled={loading}>Show more</button>
	{/if}
</main>

<style>
	.back {
		margin: 0 0 0.5rem -0.6rem;
	}
	.eyebrow {
		margin-bottom: 0.35rem;
	}
	h1 {
		margin-bottom: 1rem;
	}
	.search {
		position: relative;
		display: flex;
		align-items: center;
		max-width: 560px;
		margin-bottom: 0.9rem;
	}
	.search :global(svg) {
		position: absolute;
		left: 1rem;
		color: var(--fg-muted);
		pointer-events: none;
	}
	.search input {
		width: 100%;
		min-height: 48px;
		padding: 0 1rem 0 2.8rem;
		border: 1px solid var(--line);
		border-radius: 999px;
		background: var(--bg-raised);
		font-size: 1rem;
	}
	.search input:focus {
		outline: none;
		border-color: var(--fg);
	}
	.filters {
		overflow-x: auto;
		scrollbar-width: none;
		margin: 0 calc(-1 * var(--gutter)) 0.5rem;
		padding: 0 var(--gutter) 0.25rem;
	}
	.filters .segmented label {
		padding: 0 0.85rem;
		font-size: 0.875rem;
		white-space: nowrap;
	}
	.bar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 1rem;
		padding: 0.5rem 0;
		margin-bottom: 1.25rem;
		border-bottom: 1px solid var(--line);
	}
	.count {
		margin: 0;
		font-size: 0.875rem;
	}
	.sort {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		font-size: 0.875rem;
	}
	.sort .select {
		width: auto;
		min-height: 38px;
		font-size: 0.9rem;
		background-color: transparent;
		border-color: transparent;
		font-weight: 600;
	}
	.sort .select:hover {
		border-color: var(--line);
	}
	.grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
		gap: 1.6rem 1.1rem;
	}
	@media (min-width: 1024px) {
		.grid {
			grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
			gap: 2rem 1.5rem;
		}
	}
	.more {
		display: flex;
		margin: 2rem auto 0;
	}
</style>
