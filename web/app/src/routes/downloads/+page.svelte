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
	<span class="eyebrow accent">On this device</span>
	<h1>Downloads</h1>
	{#if downloads.storage}
		<div class="storage">
			<span class="meter" aria-hidden="true"><span style:width="{Math.min(100, (downloads.storage.usage / Math.max(1, downloads.storage.quota)) * 100)}%"></span></span>
			<p class="muted">
				<strong>{bytes(downloads.storage.usage)}</strong> used of {bytes(downloads.storage.quota)}.
				{downloads.storage.persisted ? 'Kept until you remove them.' : 'Your browser may clear these when space runs low.'}
			</p>
		</div>
	{/if}
	{#if running}<p class="note" role="status"><Icon name="info" size={18} /> Keep ABShare open while books download.</p>{/if}

	<div class="list">
		{#each downloads.list as d (d.id)}
			<div class="item">
				<a class="art" href="#/book/{d.id}"><Cover id={d.id} src={d.detail.cover} title={d.detail.title} size={128} book /></a>
				<div class="text">
					<a class="title serif" href="#/book/{d.id}">{d.detail.title}</a>
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
						<button class="btn soft" onclick={() => downloads.pause(d.id)}>Pause</button>
					{:else if d.state !== 'complete'}
						<button class="btn" onclick={() => downloads.resume(d.id)}>Resume</button>
					{/if}
					<button class="icon-btn" onclick={() => downloads.remove(d.id)} aria-label="Remove {d.detail.title}"><Icon name="trash" /></button>
				</div>
			</div>
		{:else}
			<div class="empty">
				<Icon name="downloaded" size={36} />
				<p class="serif">Nothing downloaded yet.</p>
				<p class="muted">Open a book and choose Download to listen without a connection.</p>
			</div>
		{/each}
	</div>
</main>

<style>
	h1 {
		margin-top: 0.3rem;
	}
	.storage {
		max-width: 560px;
		margin-bottom: 1.25rem;
	}
	.storage p {
		font-size: 0.9rem;
		margin: 0.5rem 0 0;
	}
	.storage strong {
		color: var(--fg);
	}
	.meter {
		display: block;
		height: 4px;
		border-radius: 2px;
		background: var(--line);
		overflow: hidden;
	}
	.meter span {
		display: block;
		height: 100%;
		background: var(--fg);
	}
	.note {
		display: flex;
		gap: 0.5rem;
		align-items: center;
		padding: 0.7rem 0.9rem;
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
		border-radius: var(--radius-sm);
		font-size: 0.9rem;
	}
	.list {
		border-top: 1px solid var(--line);
	}
	.item {
		display: flex;
		gap: 1rem;
		align-items: center;
		padding: 0.9rem 0;
		border-bottom: 1px solid var(--line);
	}
	.art {
		width: 60px;
		flex: none;
	}
	.text {
		flex: 1;
		display: grid;
		gap: 0.2rem;
		min-width: 0;
	}
	.title {
		color: inherit;
		text-decoration: none;
		font-weight: 600;
		font-size: 1.05rem;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.small {
		font-size: 0.82rem;
	}
	progress {
		width: 100%;
		height: 4px;
		appearance: none;
		border: 0;
		border-radius: 2px;
		background: var(--line);
		overflow: hidden;
	}
	progress::-webkit-progress-bar {
		background: var(--line);
	}
	progress::-webkit-progress-value {
		background: var(--fg);
	}
	progress::-moz-progress-bar {
		background: var(--fg);
	}
	.acts {
		display: flex;
		align-items: center;
		gap: 0.25rem;
	}
	.empty {
		display: grid;
		justify-items: center;
		text-align: center;
		padding: 3rem 1rem;
		color: var(--fg-muted);
	}
	.empty .serif {
		font-size: 1.3rem;
		color: var(--fg);
		margin: 0.75rem 0 0.25rem;
	}
</style>
