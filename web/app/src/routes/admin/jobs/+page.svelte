<script lang="ts">
	// Media jobs: what each lane is doing, playable copies being built or failed, and the
	// transcode cache. Retrying a copy and clearing the cache need a step-up.
	import { adminApi, type JobsView } from '$lib/api/admin';
	import { messageOf } from '$lib/api/http';
	import { session } from '$lib/api/session.svelte';
	import { bytes } from '$lib/format';
	import AdminShell from '$lib/components/AdminShell.svelte';
	import { withStepUp } from '$lib/stepup.svelte';

	let v = $state<JobsView | null>(null);
	let error = $state('');
	let note = $state('');
	async function reload() {
		try {
			v = await adminApi.jobs();
		} catch (e) {
			error = messageOf(e);
		}
	}
	$effect(() => {
		if (session.status !== 'signed-in' || session.user?.role !== 'admin') return;
		void reload();
		const t = setInterval(() => void reload(), 5000);
		return () => clearInterval(t);
	});
	async function act(fn: () => Promise<unknown>, done: string) {
		error = note = '';
		try {
			await withStepUp(fn);
			note = done;
			await reload();
		} catch (e) {
			error = messageOf(e);
		}
	}
	const LANES = [
		['listening', 'Listening now', 'Data-saver audio someone is waiting for'],
		['background', 'Background', 'Read-ahead and downloads'],
		['copies', 'Playable copies', 'Joining multi-part books into one file'],
	] as const;
	const when = (at: number) => new Date(at * 1000).toLocaleString('en-GB', { dateStyle: 'medium', timeStyle: 'short' });
</script>

<AdminShell title="Jobs">
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#if note}<p class="ok" role="status">{note}</p>{/if}
	{#if v}
		<dl class="stats">
			{#each LANES as [key, label, hint] (key)}
				<div title={hint}><dt>{label}</dt><dd>{v.lanes[key].busy} of {v.lanes[key].of}</dd></div>
			{/each}
			<div><dt>Read-aheads</dt><dd>{v.read_aheads}</dd></div>
			<div><dt>Transcode cache</dt><dd>{bytes(v.cache_bytes)}</dd></div>
		</dl>

		<h2>Being built</h2>
		{#each v.building as b (b.id)}
			<p class="item"><a href="#/book/{b.id}">{b.title}</a></p>
		{:else}
			<p class="muted">No playable copies are being built.</p>
		{/each}

		<h2>Failed copies</h2>
		{#each v.failed as f (f.id)}
			<div class="row item">
				<p class="spacer"><a href="#/book/{f.id}">{f.title}</a><br /><span class="muted">Failed {f.failures} {f.failures === 1 ? 'time' : 'times'}; tries again after {when(f.retry_at)}.</span></p>
				<button class="btn soft" onclick={() => act(() => adminApi.retryCopy(f.id), `${f.title} will be tried again when it’s next played.`)}>Retry now</button>
			</div>
		{:else}
			<p class="muted">No failed copies.</p>
		{/each}

		<h2>Cache</h2>
		<div class="row">
			<p class="spacer muted">Data-saver audio is kept so seeking and downloads are quick, within the configured limit. Clearing it removes audio nobody has played in the last 10 minutes; it is made again when needed. Playable copies are kept.</p>
			<button class="btn soft" onclick={() => act(() => adminApi.clearCache(), 'Data-saver cache cleared.')}>Clear data-saver cache</button>
		</div>
	{/if}
</AdminShell>

<style>
	.stats {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
		margin: 0 0 2rem;
		border-top: 1px solid var(--line);
		border-left: 1px solid var(--line);
	}
	.stats div {
		padding: 1rem 1.1rem;
		border-right: 1px solid var(--line);
		border-bottom: 1px solid var(--line);
	}
	dt {
		color: var(--fg-muted);
		font-size: 0.72rem;
		font-weight: 600;
		letter-spacing: 0.12em;
		text-transform: uppercase;
	}
	dd {
		margin: 0.35rem 0 0;
		font-family: var(--font-display);
		font-size: 1.7rem;
		font-weight: 600;
		line-height: 1.1;
	}
	h2 {
		margin: 2rem 0 0.75rem;
		padding-top: 1.25rem;
		border-top: 1px solid var(--line);
	}
	.row {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem 1rem;
		align-items: center;
	}
	.spacer {
		flex: 1 1 260px;
	}
	.item {
		margin: 0;
		padding: 0.6rem 0;
		border-bottom: 1px solid var(--line);
	}
	.item a {
		font-family: var(--font-display);
		font-weight: 600;
		color: inherit;
	}
	.ok {
		border: 1px solid var(--line);
		padding: 0.8rem 1rem;
		border-radius: var(--radius-sm);
	}
</style>
