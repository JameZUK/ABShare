<script lang="ts">
	// Downloads (SPEC §7.2): state and size of each book, storage used against the quota,
	// whether the browser keeps it, and remove.
	import { bytes } from '$lib/format';
	import { downloads } from '$lib/offline/downloads.svelte';
	import Cover from '$lib/components/Cover.svelte';
	import Icon from '$lib/components/Icon.svelte';

	$effect(() => void downloads.refreshStorage());

	const running = $derived(downloads.list.some((d) => d.state === 'downloading'));
	const STATE: Record<string, string> = { queued: 'Waiting', downloading: 'Downloading', paused: 'Paused', complete: 'Downloaded', error: 'Failed' };
</script>

<main class="page">
	<h1>Downloads</h1>
	{#if downloads.storage}
		<p class="muted">
			{bytes(downloads.storage.usage)} used of {bytes(downloads.storage.quota)}.
			{downloads.storage.persisted ? 'Kept until you remove them.' : 'Your browser may clear these when space runs low.'}
		</p>
	{/if}
	{#if running}<p class="note" role="status"><Icon name="info" size={18} /> Keep ABShare open while books download.</p>{/if}

	{#each downloads.list as d (d.id)}
		<div class="item">
			<a class="art" href="#/book/{d.id}"><Cover id={d.id} src={d.detail.cover} title={d.detail.title} size={128} /></a>
			<div class="text">
				<a href="#/book/{d.id}"><strong>{d.detail.title}</strong></a>
				<span class="muted small">
					{STATE[d.state]}{d.quality === 'original' ? '' : ` · data saver ${d.quality} kb/s`}
					· {d.total ? `${bytes(d.next)} of ${bytes(d.total)}` : 'size not known yet'}
					{#if d.state === 'downloading' && downloads.rate}· {bytes(downloads.rate)}/s{/if}
				</span>
				{#if d.state !== 'complete' && d.total}
					<progress max={d.total} value={d.next}></progress>
				{/if}
				{#if d.error}<span class="error small">{d.error}</span>{/if}
			</div>
			<div class="acts">
				{#if d.state === 'downloading' || d.state === 'queued'}
					<button class="btn plain" onclick={() => downloads.pause(d.id)}>Pause</button>
				{:else if d.state !== 'complete'}
					<button class="btn soft" onclick={() => downloads.resume(d.id)}>Resume</button>
				{/if}
				<button class="icon-btn" onclick={() => downloads.remove(d.id)} aria-label="Remove {d.detail.title}"><Icon name="trash" /></button>
			</div>
		</div>
	{:else}
		<p class="muted">Nothing downloaded. Open a book and choose Download to listen without a connection.</p>
	{/each}
</main>

<style>
	.note {
		display: flex;
		gap: 0.5rem;
		align-items: center;
		padding: 0.6rem 0.8rem;
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
		border-radius: var(--radius-sm);
	}
	.item {
		display: flex;
		gap: 0.9rem;
		align-items: center;
		padding: 0.75rem 0;
		border-bottom: 1px solid var(--line);
	}
	.art {
		width: 64px;
		flex: none;
	}
	.text {
		flex: 1;
		display: grid;
		gap: 0.2rem;
		min-width: 0;
	}
	.text a {
		color: inherit;
		text-decoration: none;
	}
	.small {
		font-size: 0.85rem;
	}
	progress {
		width: 100%;
		accent-color: var(--accent);
	}
	.acts {
		display: flex;
		align-items: center;
		gap: 0.25rem;
	}
</style>
