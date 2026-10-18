<script lang="ts">
	// Settings → History (SPEC §5.6, §7.2): your listening time, recent sessions, a switch to
	// stop recording, and Clear history behind ⋮ and a confirmation. Only you can see this.
	import { messageOf } from '$lib/api/http';
	import { api, post, session } from '$lib/api/session.svelte';
	import { listened, when, type History, type HistorySession } from '$lib/history';
	import { names } from '$lib/format';
	import Cover from '$lib/components/Cover.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import Sheet from '$lib/components/Sheet.svelte';

	let data = $state<History | null>(null);
	let sessions = $state<HistorySession[]>([]);
	let next = $state<string | null>(null);
	let error = $state('');
	let busy = $state(false);
	let moreOpen = $state(false);
	let confirmOpen = $state(false);

	async function reload() {
		try {
			const h = await api<History>('v1/me/history');
			data = h;
			sessions = h.sessions;
			next = h.next;
		} catch (e) {
			error = messageOf(e);
		}
	}
	$effect(() => {
		if (session.status === 'signed-in') void reload();
	});

	async function loadMore() {
		if (!next || busy) return;
		busy = true;
		try {
			const h = await api<History>(`v1/me/history?cursor=${encodeURIComponent(next)}`);
			sessions = [...sessions, ...h.sessions];
			next = h.next;
		} catch (e) {
			error = messageOf(e);
		} finally {
			busy = false;
		}
	}

	async function setEnabled(enabled: boolean) {
		error = '';
		busy = true;
		try {
			await post('v1/me/history', { enabled }, 'PATCH');
			await reload();
		} catch (e) {
			error = messageOf(e);
		} finally {
			busy = false;
		}
	}

	async function clear() {
		error = '';
		busy = true;
		try {
			await api('v1/me/history', { method: 'DELETE' });
			confirmOpen = false;
			await reload();
		} catch (e) {
			error = messageOf(e);
		} finally {
			busy = false;
		}
	}
</script>

<main class="page narrow">
	<div class="row">
		<button class="icon-btn" onclick={() => history.back()} aria-label="Back"><Icon name="back" /></button>
		<span class="spacer"></span>
		<button class="icon-btn" onclick={() => (moreOpen = true)} aria-label="More"><Icon name="more" /></button>
	</div>
	<h1>History</h1>
	<p class="muted">Your listening time and recent sessions. Only you can see this; administrators can’t. Sessions are kept for 13 months.</p>
	{#if error}<p class="error" role="alert">{error}</p>{/if}

	{#if data}
		<dl class="totals">
			<div><dt>Last 7 days</dt><dd>{listened(data.totals.week_seconds)}</dd></div>
			<div><dt>Last 30 days</dt><dd>{listened(data.totals.month_seconds)}</dd></div>
			<div><dt>All time</dt><dd>{listened(data.totals.all_seconds)}</dd></div>
			<div><dt>Books finished</dt><dd>{data.totals.books_finished.toLocaleString('en-GB')}</dd></div>
		</dl>

		<label class="choice">
			<input type="checkbox" checked={data.enabled} disabled={busy} onchange={(e) => setEnabled(e.currentTarget.checked)} />
			<span><strong>Keep listening history</strong><br /><span class="muted">When this is off, nothing new is recorded. Your place in each book is still saved.</span></span>
		</label>

		<h2 class="sessions">Recent sessions</h2>
		{#if sessions.length === 0}
			<p class="muted">{data.enabled ? 'Nothing yet. Sessions appear here as you listen.' : 'History is off.'}</p>
		{/if}
		{#each sessions as s (s.id)}
			<a class="item row" href="#/book/{s.book.id}">
				<span class="cover"><Cover id={s.book.id} src={s.book.cover} title={s.book.title} size={128} book /></span>
				<span class="spacer text">
					<strong class="serif">{s.book.title}</strong>
					{#if s.book.authors.length}<span class="muted small">{names(s.book.authors)}</span>{/if}
					<span class="muted small">{when(s.started_at)} · {listened(s.seconds)}</span>
				</span>
			</a>
		{/each}
		{#if next}
			<button class="btn soft more" onclick={loadMore} disabled={busy}>Show more</button>
		{/if}
	{/if}
</main>

<Sheet bind:open={moreOpen} title="History">
	<button class="btn danger" onclick={() => { moreOpen = false; confirmOpen = true; }}><Icon name="trash" /> Clear history…</button>
</Sheet>

<Sheet bind:open={confirmOpen} title="Clear history?">
	<p>This removes all your listening sessions and resets your listening time. Your place in each book and your finished books are kept. It can’t be undone.</p>
	<div class="choices">
		<button class="btn danger" onclick={clear} disabled={busy}>Clear history</button>
		<button class="btn" onclick={() => (confirmOpen = false)}>Cancel</button>
	</div>
</Sheet>

<style>
	/* Totals as a row of figures, divided by hairlines. */
	.totals {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		margin: 1.5rem 0;
		border-top: 1px solid var(--line);
		border-bottom: 1px solid var(--line);
	}
	@media (min-width: 560px) {
		.totals {
			grid-template-columns: repeat(4, minmax(0, 1fr));
		}
	}
	.totals div {
		padding: 1rem 0.75rem 1rem 0;
	}
	.totals div + div {
		padding-left: 0.9rem;
		border-left: 1px solid var(--line);
	}
	@media (max-width: 559px) {
		.totals div:nth-child(3) {
			padding-left: 0;
			border-left: 0;
		}
		.totals div:nth-child(n + 3) {
			border-top: 1px solid var(--line);
		}
	}
	.totals dt {
		font-size: 0.72rem;
		font-weight: 600;
		letter-spacing: 0.12em;
		text-transform: uppercase;
		color: var(--fg-muted);
	}
	.totals dd {
		margin: 0.3rem 0 0;
		font-family: var(--font-display);
		font-size: 1.6rem;
		font-weight: 600;
		line-height: 1.1;
	}
	.choice {
		display: flex;
		gap: 0.85rem;
		align-items: flex-start;
		padding: 0.5rem 0;
		min-height: 44px;
	}
	.choice input {
		margin-top: 0.25rem;
		width: 1.2rem;
		height: 1.2rem;
		flex: none;
	}
	.sessions {
		margin-top: 2rem;
		padding-bottom: 0.6rem;
		border-bottom: 1px solid var(--line);
	}
	.item {
		gap: 0.9rem;
		min-height: 72px;
		padding: 0.6rem 0;
		border-bottom: 1px solid var(--line);
		color: inherit;
		text-decoration: none;
	}
	.cover {
		width: 48px;
		flex: none;
	}
	.text {
		display: grid;
		min-width: 0;
	}
	.text strong {
		font-size: 1.02rem;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.small {
		font-size: 0.82rem;
	}
	.more {
		display: flex;
		margin: 1.5rem auto 0;
	}
	.choices {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
	}
</style>
