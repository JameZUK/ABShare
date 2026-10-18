<script lang="ts">
	// One person's listening: totals, each book they've started with its status and time
	// heard, and their recent sessions. Never positions in a book: those stay theirs.
	import { page } from '$app/state';
	import { adminApi, type PersonListeningDetail } from '$lib/api/admin';
	import { messageOf } from '$lib/api/http';
	import { session } from '$lib/api/session.svelte';
	import { duration } from '$lib/format';
	import { ago, byDay, timeOfDay } from '$lib/when';
	import AdminShell from '$lib/components/AdminShell.svelte';
	import Cover from '$lib/components/Cover.svelte';

	const uid = $derived(Number(page.params.uid));
	let v = $state<PersonListeningDetail | null>(null);
	let error = $state('');
	$effect(() => {
		const id = uid;
		if (session.status !== 'signed-in' || session.user?.role !== 'admin') return;
		v = null;
		adminApi
			.personListening(id)
			.then((r) => (v = r))
			.catch((e) => (error = messageOf(e)));
	});
	const hours = (s: number) => (s < 3600 ? `${Math.round(s / 60)} min` : `${(s / 3600).toFixed(1)} h`);
	const STATUS = { finished: 'Finished', in_progress: 'Listening', abandoned: 'Abandoned' } as const;
	const days = $derived(byDay(v?.sessions ?? [], (s) => s.started_at));
</script>

<AdminShell title={v ? v.name : 'Listening'}>
	<p><a href="#/admin/listening">← Everyone’s listening</a></p>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#if v}
		<p class="muted">{v.last_listened_at ? `Last listened ${ago(v.last_listened_at)}` : 'Not listened yet'}{v.history_on ? '' : ' · history is off, so new listening isn’t counted'}</p>
		<dl class="stats">
			<div><dt>Last 7 days</dt><dd>{hours(v.totals.week_seconds)}</dd></div>
			<div><dt>Last 30 days</dt><dd>{hours(v.totals.month_seconds)}</dd></div>
			<div><dt>In all</dt><dd>{hours(v.totals.all_seconds)}</dd></div>
			<div><dt>Finished</dt><dd>{v.totals.finished}</dd></div>
			<div><dt>Listening</dt><dd>{v.totals.in_progress}</dd></div>
			<div><dt>Abandoned</dt><dd>{v.totals.abandoned}</dd></div>
		</dl>

		<h2>Books</h2>
		{#if !v.books.length}<p class="muted">No books started yet.</p>{/if}
		<ul class="books">
			{#each v.books as b (b.book.id)}
				<li>
					<span class="thumb">{#if b.book.title}<Cover id={b.book.id} src={b.book.cover} title={b.book.title} size={128} book />{/if}</span>
					<span class="text">
						<span class="serif title">{b.book.title ?? 'A book no longer in the library'}</span>
						<span class="muted small">{b.book.authors.join(', ')}{b.book.authors.length ? ' · ' : ''}{hours(b.heard_seconds)} heard · {ago(b.last_at)}</span>
					</span>
					<span class="status {b.status}">{STATUS[b.status]}</span>
				</li>
			{/each}
		</ul>

		<h2>Recent sessions</h2>
		{#if !v.sessions.length}<p class="muted">None recorded.</p>{/if}
		{#each days as d (d.day)}
			<h3 class="eyebrow">{d.day}</h3>
			<ul class="sessions">
				{#each d.items as s, i (i)}
					<li><span>{timeOfDay(s.started_at)} – {timeOfDay(s.ended_at)}</span><span class="serif">{s.book.title ?? 'A book no longer in the library'}</span><span class="muted">{duration(s.seconds * 1000)}</span></li>
				{/each}
			</ul>
		{/each}
	{/if}
</AdminShell>

<style>
	.stats {
		display: grid;
		grid-template-columns: repeat(3, minmax(0, 1fr));
		margin: 1rem 0 2rem;
		border-top: 1px solid var(--line);
		border-left: 1px solid var(--line);
	}
	@media (min-width: 720px) {
		.stats {
			grid-template-columns: repeat(6, minmax(0, 1fr));
		}
	}
	.stats div {
		padding: 0.8rem 0.9rem;
		border-right: 1px solid var(--line);
		border-bottom: 1px solid var(--line);
	}
	.stats dt {
		font-size: 0.7rem;
		font-weight: 600;
		letter-spacing: 0.12em;
		text-transform: uppercase;
		color: var(--fg-muted);
	}
	.stats dd {
		margin: 0.25rem 0 0;
		font-family: var(--font-display);
		font-size: 1.4rem;
		font-weight: 600;
	}
	h2 {
		margin-top: 1.5rem;
	}
	.books,
	.sessions {
		list-style: none;
		margin: 0 0 1rem;
		padding: 0;
	}
	.books li {
		display: flex;
		align-items: center;
		gap: 0.9rem;
		padding: 0.7rem 0;
		border-top: 1px solid var(--line);
	}
	.thumb {
		width: 44px;
		flex: none;
	}
	.text {
		display: grid;
		flex: 1;
		min-width: 0;
	}
	.title {
		font-weight: 600;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.small {
		font-size: 0.82rem;
	}
	.status {
		font-size: 0.75rem;
		font-weight: 600;
		padding: 0.2rem 0.6rem;
		border-radius: 999px;
		border: 1px solid var(--line);
		white-space: nowrap;
	}
	.status.finished {
		background: var(--fg);
		color: var(--bg);
		border-color: var(--fg);
	}
	.status.abandoned {
		color: var(--accent);
		border-color: currentColor;
	}
	.sessions li {
		display: grid;
		grid-template-columns: 7.5rem minmax(0, 1fr) auto;
		gap: 0.75rem;
		padding: 0.5rem 0;
		border-top: 1px solid var(--line);
		font-size: 0.9rem;
		font-variant-numeric: tabular-nums;
	}
	.sessions .serif {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
</style>
