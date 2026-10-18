<script lang="ts">
	// Recommend books: pick up to 10, say why, and send them to everyone or to chosen people.
	// They appear at the top of each recipient's Home until dismissed or started. Sending and
	// withdrawing need a step-up.
	import { adminApi, type AdminRecommendation, type AdminUser } from '$lib/api/admin';
	import { messageOf } from '$lib/api/http';
	import { api, session } from '$lib/api/session.svelte';
	import type { BookSummary, Page, Tile } from '$lib/api/types';
	import { names } from '$lib/format';
	import AdminShell from '$lib/components/AdminShell.svelte';
	import Cover from '$lib/components/Cover.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import { withStepUp } from '$lib/stepup.svelte';
	import { NOTE_CHARS, NOTE_WORDS, countChars, countWords } from '$lib/words';

	const MAX_BOOKS = 10;

	let list = $state<AdminRecommendation[]>([]);
	let users = $state<AdminUser[]>([]);
	let error = $state('');
	let ok = $state('');
	let busy = $state(false);

	let q = $state('');
	let results = $state<BookSummary[]>([]);
	let picked = $state<BookSummary[]>([]);
	let note = $state('');
	let audience = $state<'everyone' | 'people'>('everyone');
	let chosen = $state<number[]>([]);

	const words = $derived(countWords(note));
	const noteOk = $derived(words >= 1 && words <= NOTE_WORDS && countChars(note) <= NOTE_CHARS);
	const canSend = $derived(picked.length > 0 && noteOk && (audience === 'everyone' || chosen.length > 0) && !busy);
	const people = $derived(users.filter((u) => !u.disabled));

	async function reload() {
		try {
			list = await adminApi.recommendations();
		} catch (e) {
			error = messageOf(e);
		}
	}

	$effect(() => {
		if (session.status !== 'signed-in' || session.user?.role !== 'admin') return;
		void reload();
		adminApi
			.users()
			.then((u) => (users = u))
			.catch((e) => (error = messageOf(e)));
	});

	// Search the library as the admin types (after a short pause).
	let seq = 0;
	$effect(() => {
		const term = q.trim();
		const mine = ++seq;
		if (!term) {
			results = [];
			return;
		}
		const t = setTimeout(async () => {
			try {
				const r = await api<Page<Tile>>(`v1/books?${new URLSearchParams({ q: term, limit: '10', sort: 'title' })}`);
				if (mine === seq) results = r.items.filter((x): x is BookSummary => x.type === 'book');
			} catch (e) {
				if (mine === seq) error = messageOf(e);
			}
		}, 250);
		return () => clearTimeout(t);
	});

	function pick(b: BookSummary) {
		if (picked.length >= MAX_BOOKS || picked.some((p) => p.id === b.id)) return;
		picked = [...picked, b];
	}

	function toggle(id: number, on: boolean) {
		chosen = on ? [...new Set([...chosen, id])] : chosen.filter((x) => x !== id);
	}

	async function send() {
		if (!canSend) return;
		busy = true;
		error = ok = '';
		try {
			await withStepUp(() =>
				adminApi.createRecommendation({ books: picked.map((b) => b.id), note: note.trim(), everyone: audience === 'everyone', users: audience === 'everyone' ? [] : chosen }),
			);
			ok = audience === 'everyone' ? 'Sent to everyone.' : `Sent to ${chosen.length} ${chosen.length === 1 ? 'person' : 'people'}.`;
			picked = [];
			note = '';
			q = '';
			chosen = [];
			await reload();
		} catch (e) {
			error = messageOf(e);
		} finally {
			busy = false;
		}
	}

	let confirming = $state<number | null>(null);
	async function withdraw(id: number) {
		error = ok = '';
		try {
			await withStepUp(() => adminApi.withdrawRecommendation(id));
			confirming = null;
			ok = 'Withdrawn.';
			await reload();
		} catch (e) {
			error = messageOf(e);
		}
	}

	const when = (at: number) => new Date(at * 1000).toLocaleDateString('en-GB', { day: 'numeric', month: 'short', year: 'numeric' });
	const audienceLabel = (r: AdminRecommendation) => (r.everyone ? 'Everyone' : r.users.map((u) => u.name).join(', ') || 'Nobody');
</script>

<AdminShell title="Recommend">
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#if ok}<p class="ok" role="status">{ok}</p>{/if}

	<form class="compose" onsubmit={(e) => { e.preventDefault(); void send(); }}>
		<h2>Recommend a book</h2>

		<fieldset>
			<legend class="eyebrow">Books ({picked.length} of {MAX_BOOKS})</legend>
			{#if picked.length}
				<ul class="chips picked">
					{#each picked as b (b.id)}
						<li><button type="button" class="chip" aria-pressed="true" onclick={() => (picked = picked.filter((p) => p.id !== b.id))} aria-label="Remove {b.title}">{b.title} <Icon name="close" size={14} /></button></li>
					{/each}
				</ul>
			{/if}
			<label class="search">
				<span class="visually-hidden">Search for a book</span>
				<Icon name="search" size={18} />
				<input class="input" type="search" bind:value={q} placeholder="Search titles, authors, narrators" autocomplete="off" maxlength="200" disabled={picked.length >= MAX_BOOKS} />
			</label>
			{#if results.length}
				<ul class="results rules">
					{#each results as b (b.id)}
						<li>
							<button type="button" onclick={() => pick(b)} disabled={picked.some((p) => p.id === b.id) || picked.length >= MAX_BOOKS}>
								<span class="thumb"><Cover id={b.id} src={b.cover} title={b.title} size={128} /></span>
								<span class="rt"><span class="serif">{b.title}</span><span class="muted">{names(b.authors)}</span></span>
								<Icon name={picked.some((p) => p.id === b.id) ? 'check' : 'right'} size={18} />
							</button>
						</li>
					{/each}
				</ul>
			{/if}
		</fieldset>

		<label class="field">
			<span>Why you recommend it</span>
			<textarea bind:value={note} rows="3" maxlength="4000" placeholder="A line or two on why it’s worth a listen"></textarea>
		</label>
		<p class="count" class:over={words > NOTE_WORDS || countChars(note) > NOTE_CHARS} aria-live="polite">{words} / {NOTE_WORDS} words</p>

		<fieldset>
			<legend class="eyebrow">Send to</legend>
			<div class="segmented">
				<label><input type="radio" name="audience" value="everyone" bind:group={audience} />Everyone</label>
				<label><input type="radio" name="audience" value="people" bind:group={audience} />Choose people</label>
			</div>
			{#if audience === 'everyone'}
				<p class="muted hint">Everyone, including people who join later.</p>
			{:else}
				<ul class="people">
					{#each people as u (u.id)}
						<li>
							<label class="person">
								<input type="checkbox" checked={chosen.includes(u.id)} onchange={(e) => toggle(u.id, e.currentTarget.checked)} />
								<span>{u.display_name} <span class="muted">({u.username})</span></span>
							</label>
						</li>
					{/each}
				</ul>
			{/if}
		</fieldset>

		<button type="submit" class="btn" disabled={!canSend}>Send recommendation</button>
	</form>

	<section class="sent">
		<h2>Sent</h2>
		{#each list as r (r.id)}
			<article class="item">
				<div class="row head">
					<span class="eyebrow">{when(r.created_at)} · {audienceLabel(r)}</span>
					<span class="spacer"></span>
					<span class="muted small">{r.dismissed} dismissed</span>
				</div>
				<p class="titles serif">{r.books.map((b) => b.title).join(', ') || 'No books left in the library'}</p>
				{#if r.note}<p class="note">{r.note}</p>{/if}
				{#if confirming === r.id}
					<div class="row">
						<span>Withdraw this for everyone?</span>
						<button class="btn danger" onclick={() => withdraw(r.id)}>Withdraw</button>
						<button class="btn soft" onclick={() => (confirming = null)}>Keep</button>
					</div>
				{:else}
					<button class="btn soft" onclick={() => (confirming = r.id)}>Withdraw…</button>
				{/if}
			</article>
		{:else}
			<p class="muted">No recommendations yet.</p>
		{/each}
	</section>
</AdminShell>

<style>
	.compose {
		max-width: 620px;
		margin-bottom: 2.5rem;
	}
	fieldset {
		border: 0;
		padding: 0;
		margin: 0 0 1.5rem;
		min-width: 0;
	}
	legend {
		margin-bottom: 0.6rem;
	}
	.picked {
		list-style: none;
		margin: 0 0 0.75rem;
		padding: 0;
	}
	.search {
		position: relative;
		display: flex;
		align-items: center;
	}
	.search :global(svg) {
		position: absolute;
		left: 0.9rem;
		color: var(--fg-muted);
		pointer-events: none;
	}
	.search input {
		width: 100%;
		padding-left: 2.6rem;
		border-radius: 999px;
	}
	.results {
		list-style: none;
		margin: 0.5rem 0 0;
		padding: 0;
	}
	.results button {
		display: flex;
		align-items: center;
		gap: 0.8rem;
		width: 100%;
		min-height: 56px;
		padding: 0.4rem 0;
		border: 0;
		background: transparent;
		text-align: left;
	}
	.results button:disabled {
		opacity: 0.55;
		cursor: default;
	}
	.thumb {
		width: 40px;
		flex: none;
	}
	.rt {
		display: grid;
		flex: 1;
		min-width: 0;
		font-size: 0.85rem;
	}
	.rt > span {
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.rt .serif {
		font-size: 1rem;
		font-weight: 600;
	}
	.field textarea {
		width: 100%;
	}
	.count {
		margin: -0.6rem 0 1.25rem;
		font-size: 0.85rem;
		color: var(--fg-muted);
		font-variant-numeric: tabular-nums;
	}
	.count.over {
		color: var(--danger);
		font-weight: 600;
	}
	.hint {
		font-size: 0.875rem;
		margin-top: 0.6rem;
	}
	.people {
		list-style: none;
		margin: 0.75rem 0 0;
		padding: 0;
		border-top: 1px solid var(--line);
	}
	.people li {
		border-bottom: 1px solid var(--line);
	}
	.person {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		min-height: 48px;
		cursor: pointer;
	}
	.person input {
		width: 1.15rem;
		height: 1.15rem;
	}
	.sent h2 {
		padding-top: 1.5rem;
		border-top: 1px solid var(--line);
	}
	.item {
		padding: 1rem 0;
	}
	.item + .item {
		border-top: 1px solid var(--line);
	}
	.titles {
		margin: 0.3rem 0;
		font-size: 1.05rem;
		font-weight: 600;
	}
	.note {
		font-family: var(--font-display);
		font-style: italic;
		color: var(--fg-muted);
		white-space: pre-line;
		overflow-wrap: anywhere;
	}
	.small {
		font-size: 0.82rem;
	}
	.ok {
		color: var(--fg);
		font-weight: 600;
	}
</style>
