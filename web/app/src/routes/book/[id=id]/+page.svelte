<script lang="ts">
	// A book (SPEC §7.2): cover, title, series, people, length and time left, description and
	// chapters. Play/Resume and Download; the rest is in ⋮.
	import { page } from '$app/state';
	import { api, post, session } from '$lib/api/session.svelte';
	import { messageOf } from '$lib/api/http';
	import type { BookDetail, Me } from '$lib/api/types';
	import { bytes, clock, duration, names, timeLeft } from '$lib/format';
	import { downloads, sizeOf } from '$lib/offline/downloads.svelte';
	import { sync } from '$lib/offline/sync.svelte';
	import { player } from '$lib/player/player.svelte';
	import { settings } from '$lib/settings.svelte';
	import type { Quality } from '$lib/offline/db';
	import Cover from '$lib/components/Cover.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import Sheet from '$lib/components/Sheet.svelte';
	import EditDetails from '$lib/components/EditDetails.svelte';
	import PeopleLinks from '$lib/components/PeopleLinks.svelte';
	import ListeningTimeline from '$lib/components/ListeningTimeline.svelte';
	import Reviews from '$lib/components/Reviews.svelte';
	import Stars from '$lib/components/Stars.svelte';
	import StepUpSheet from '$lib/components/StepUpSheet.svelte';

	const id = $derived(page.params.id ?? '');
	let book = $state<BookDetail | null>(null);
	let error = $state('');
	let canDownload = $state(true);
	let canSave = $state(false);
	let moreOpen = $state(false);
	let timelineOpen = $state(false);
	let dlOpen = $state(false);
	let marksOpen = $state(false);
	let editOpen = $state(false);
	let reloadKey = $state(0);
	let dlQuality = $state<Quality>('original');
	let dlSize = $state<number | null>(null);
	let me = $state<Me | null>(null);
	// Updated by the reviews section when someone saves or removes a review.
	let summary = $state<{ average: number | null; count: number } | null>(null);
	const rating = $derived(summary ?? book?.rating ?? null);

	$effect(() => {
		const bid = id;
		void reloadKey;
		book = null;
		summary = null;
		error = '';
		void (async () => {
			try {
				book = session.status === 'signed-in' ? await api<BookDetail>(`v1/books/${bid}`) : ((await downloads.detail(bid)) ?? null);
				if (!book) error = 'This book isn’t on this device. Connect to see it.';
			} catch (e) {
				book = (await downloads.detail(bid)) ?? null;
				if (!book) error = messageOf(e);
			}
		})();
	});

	$effect(() => {
		if (session.status !== 'signed-in') return;
		api<Me>('v1/me')
			.then((m) => {
				me = m;
				canDownload = m.permissions.offline;
				canSave = m.permissions.save_files;
			})
			.catch(() => {});
	});

	const isOpen = $derived(player.book?.id === id);
	const position = $derived(isOpen ? player.position : (book?.progress && !book.progress.finished ? book.progress.position_ms : 0));
	const dl = $derived(downloads.byId(id));
	const paragraphs = $derived((book?.description ?? '').split(/\n{2,}/).filter((p) => p.trim()));
	const seriesGroup = $derived(book?.groups.find((g) => g.kind === 'series' || g.kind === 'show'));

	function play() {
		if (!book) return;
		if (isOpen) player.toggle();
		else void player.open(book, { play: true });
	}

	async function openDownload() {
		dlQuality = settings.prefs.downloadQuality === 'saver' ? settings.prefs.saverKbps : 'original';
		dlOpen = true;
		await refreshSize();
	}

	async function refreshSize() {
		dlSize = null;
		dlSize = await sizeOf(id, dlQuality);
	}

	async function startDownload() {
		if (!book) return;
		await downloads.add(book, dlQuality);
		dlOpen = false;
	}

	async function mark(action: 'finished' | 'unfinished' | 'restart') {
		if (!book) return;
		if (isOpen) await player.mark(action);
		else {
			await sync.record(book.id, action);
			if (action === 'finished') player.finishedNow = { id: book.id, title: book.title, cover: book.cover };
		}
		book = { ...book, progress: { position_ms: action === 'restart' ? 0 : position, finished: action === 'finished', updated_at: Date.now() } };
		moreOpen = false;
	}

	// Editing a bookmark's note, in place in the list.
	let editing = $state<number | null>(null);
	let noteText = $state('');
	function editMark(markId: number, note: string | null) {
		editing = markId;
		noteText = note ?? '';
	}
	async function saveNote(markId: number) {
		if (!book) return;
		const note = noteText.trim().slice(0, 2000);
		try {
			await post(`v1/bookmarks/${markId}`, { note }, 'PATCH');
			book = { ...book, bookmarks: book.bookmarks.map((m) => (m.id === markId ? { ...m, note } : m)) };
			editing = null;
		} catch (e) {
			error = messageOf(e);
		}
	}

	async function removeMark(markId: number) {
		if (!book) return;
		try {
			await api(`v1/bookmarks/${markId}`, { method: 'DELETE' });
			book = { ...book, bookmarks: book.bookmarks.filter((m) => m.id !== markId) };
		} catch (e) {
			error = messageOf(e);
		}
	}
</script>

<main class="page">
	<button class="icon-btn back" onclick={() => history.back()} aria-label="Back"><Icon name="back" /></button>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#if book}
		<div class="head">
			<div class="art"><Cover id={book.id} src={book.cover} title={book.title} size={640} book /></div>
			<div class="info">
				{#if book.series && seriesGroup}
					<a class="eyebrow accent series" href="#/group/{seriesGroup.id}">{book.series.name}{book.series.position ? ` · Book ${book.series.position}` : ''}</a>
				{/if}
				<h1>{book.title}</h1>
				{#if book.subtitle}<p class="subtitle serif">{book.subtitle}</p>{/if}
				{#if book.authors?.length}<p class="by">By <PeopleLinks people={book.authors} kind="author" /></p>{/if}
				{#if book.narrators?.length}<p class="by muted">Read by <PeopleLinks people={book.narrators} kind="narrator" /></p>{/if}
				<p class="meta muted">
					<span>{duration(book.duration_ms)}</span>
					{#if book.progress?.finished}<span>Finished</span>{:else if position > 0}<span>{timeLeft(book.duration_ms, position)}</span>{/if}
					{#if book.chapters.length}<span>{book.chapters.length} chapters</span>{/if}
				</p>
				{#if rating}
					<p class="rating">
						{#if rating.count && rating.average !== null}
							<a href="#reviews" onclick={(e) => { e.preventDefault(); document.getElementById('reviews')?.scrollIntoView({ behavior: 'smooth' }); }}>
								<Stars value={rating.average} size={16} />
								<span class="avg">{rating.average.toFixed(1)}</span>
								<span class="muted">· {rating.count} {rating.count === 1 ? 'review' : 'reviews'}</span>
							</a>
						{:else}
							<span class="muted">No reviews yet</span>
						{/if}
					</p>
				{/if}
				{#if position > 0 && !book.progress?.finished}
					<span class="progress" aria-hidden="true"><span style:width="{Math.min(100, (position / Math.max(1, book.duration_ms)) * 100)}%"></span></span>
				{/if}
				<div class="actions">
					<button class="btn" onclick={play}>
						<Icon name={isOpen && player.playing ? 'pause' : 'play'} size={18} />
						{isOpen && player.playing ? 'Pause' : position > 0 ? `Resume from ${clock(position)}` : 'Play'}
					</button>
					{#if canDownload}
						{#if !dl}
							<button class="btn soft" onclick={openDownload} disabled={session.status !== 'signed-in'}><Icon name="download" size={18} /> Download</button>
						{:else if dl.state === 'complete'}
							<span class="btn soft" aria-disabled="true"><Icon name="downloaded" size={18} /> Downloaded</span>
						{:else}
							<a class="btn soft" href="#/downloads"><Icon name="download" size={18} /> {dl.total ? Math.floor((dl.next / dl.total) * 100) : 0}%</a>
						{/if}
					{/if}
					<button class="icon-btn more" onclick={() => (moreOpen = true)} aria-label="More"><Icon name="more" /></button>
				</div>
				{#if book.shelves?.length}
					<p class="chips genres">{#each book.shelves as g (g.id)}<a class="chip" href="#/genre/{g.id}">{g.name}</a>{/each}</p>
				{/if}
			</div>
		</div>

		{#if paragraphs.length}
			<section class="about">
				<h2>About this book</h2>
				{#each paragraphs as p, i (i)}<p class="desc">{p}</p>{/each}
			</section>
		{/if}

		<Reviews bookId={book.id} onsummary={(v) => (summary = v)} />

		{#if book.chapters.length}
			<section>
				<h2>Chapters</h2>
				<ol class="chapters">
					{#each book.chapters as c, i (i)}
						<li>
							<button class:current={isOpen && player.chapter === i} aria-current={isOpen && player.chapter === i ? 'true' : undefined} onclick={() => book && player.open(book, { play: true, at: c.start_ms })}>
								<span class="num">{i + 1}</span>
								<span class="ct">{c.title}</span>
								<span class="muted when">{clock(c.start_ms)}</span>
							</button>
						</li>
					{/each}
				</ol>
			</section>
		{/if}
	{/if}
</main>

<Sheet bind:open={moreOpen} title="More">
	<div class="menu">
		{#if book?.progress?.finished}
			<button class="btn soft" onclick={() => mark('unfinished')}>Mark not finished</button>
		{:else}
			<button class="btn soft" onclick={() => mark('finished')}>Mark finished</button>
		{/if}
		<button class="btn soft" onclick={() => mark('restart')}>Start again</button>
		<button class="btn soft" onclick={() => { moreOpen = false; timelineOpen = true; }}>Where was I? (your listening)</button>
		<button class="btn soft" onclick={() => { moreOpen = false; marksOpen = true; }}>Bookmarks ({book?.bookmarks.length ?? 0})</button>
		{#if session.user?.role === 'admin' && session.status === 'signed-in'}
			<button class="btn soft" onclick={() => { moreOpen = false; editOpen = true; }}>Edit details</button>
		{/if}
		{#if canSave && session.status === 'signed-in'}
			<a class="btn soft" href="v1/media/{id}/file" download onclick={() => (moreOpen = false)}>Save file</a>
		{/if}
		{#if dl}
			<button class="btn danger" onclick={() => { void downloads.remove(id); moreOpen = false; }}>Remove download</button>
		{/if}
	</div>
</Sheet>

<Sheet bind:open={dlOpen} title="Download">
	<p>Listen without a connection. Keep ABShare open while it downloads.</p>
	<div class="menu">
		<label class="choice"><input type="radio" bind:group={dlQuality} value="original" onchange={refreshSize} /> Original quality</label>
		<label class="choice"><input type="radio" bind:group={dlQuality} value={settings.prefs.saverKbps} onchange={refreshSize} /> Data saver ({settings.prefs.saverKbps} kb/s, smaller)</label>
	</div>
	<p class="muted">
		Size: {dlSize === null ? 'checking…' : bytes(dlSize)}
		{#if downloads.storage}· {bytes(Math.max(0, downloads.storage.quota - downloads.storage.usage))} free for ABShare{/if}
	</p>
	{#if dlSize !== null && downloads.storage && dlSize > downloads.storage.quota - downloads.storage.usage}
		<p class="error">There isn’t enough space on this device.</p>
	{/if}
	<button class="btn" onclick={startDownload} disabled={book?.media.preparing && dlSize === null}>Download</button>
</Sheet>

{#if book}<EditDetails {book} bind:open={editOpen} onsaved={() => reloadKey++} />{/if}
{#if book}<ListeningTimeline {book} bind:open={timelineOpen} />{/if}
{#if session.user?.role === 'admin'}<StepUpSheet {me} />{/if}

<Sheet bind:open={marksOpen} title="Bookmarks">
	{#if !book?.bookmarks.length}
		<p class="muted">No bookmarks yet. Add one from the player.</p>
	{:else}
		<ul class="marks">
			{#each book.bookmarks as m (m.id)}
				<li class="row">
					{#if editing === m.id}
						<form class="note-edit" onsubmit={(e) => { e.preventDefault(); void saveNote(m.id); }}>
							<label for="note-{m.id}">Note for {clock(m.position_ms)}</label>
							<textarea id="note-{m.id}" bind:value={noteText} maxlength="2000" rows="3"></textarea>
							<div class="note-actions">
								<button type="submit" class="btn">Save</button>
								<button type="button" class="btn soft" onclick={() => (editing = null)}>Cancel</button>
							</div>
						</form>
					{:else}
						<button class="mark" onclick={() => { if (book) void player.open(book, { play: true, at: m.position_ms }); marksOpen = false; }}>
							<strong>{clock(m.position_ms)}</strong>
							{#if m.note}<span class="muted">{m.note}</span>{/if}
						</button>
						<button class="icon-btn" onclick={() => editMark(m.id, m.note)} aria-label="{m.note ? 'Edit the note' : 'Add a note'} at {clock(m.position_ms)}"><Icon name="pencil" /></button>
						<button class="icon-btn" onclick={() => removeMark(m.id)} aria-label="Delete bookmark at {clock(m.position_ms)}"><Icon name="trash" /></button>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
</Sheet>

<style>
	.back {
		margin: 0 0 0.5rem -0.6rem;
	}
	.head {
		display: grid;
		gap: 1.75rem;
		grid-template-columns: minmax(0, 1fr);
		margin-bottom: 2.25rem;
	}
	.art {
		width: min(64vw, 260px);
		margin: 0 auto;
	}
	.info {
		text-align: center;
	}
	.series {
		text-decoration: none;
		margin-bottom: 0.5rem;
	}
	h1 {
		font-size: clamp(1.8rem, 6vw, 2.5rem);
		margin-bottom: 0.25rem;
	}
	.subtitle {
		font-style: italic;
		color: var(--fg-muted);
		font-size: 1.1rem;
		margin: 0 0 0.5rem;
	}
	.by {
		margin: 0.15rem 0 0;
	}
	.by :global(a) {
		color: inherit;
		text-decoration: underline;
		text-decoration-color: var(--line);
	}
	.meta {
		display: flex;
		justify-content: center;
		flex-wrap: wrap;
		gap: 0 0.5rem;
		margin: 0.75rem 0 0;
		font-size: 0.9rem;
	}
	.meta span + span::before {
		content: '·';
		margin-right: 0.5rem;
	}
	.rating {
		margin: 0.5rem 0 0;
		font-size: 0.9rem;
	}
	.rating a {
		display: inline-flex;
		align-items: center;
		gap: 0.4rem;
		color: inherit;
		text-decoration: none;
	}
	.rating .avg {
		font-weight: 600;
	}
	.progress {
		display: block;
		height: 2px;
		max-width: 260px;
		margin: 0.75rem auto 0;
		background: var(--line);
	}
	.progress span {
		display: block;
		height: 100%;
		background: var(--fg);
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		justify-content: center;
		gap: 0.6rem;
		align-items: center;
		margin-top: 1.25rem;
	}
	.genres {
		justify-content: center;
		margin: 1.25rem 0 0;
	}
	.genres .chip {
		min-height: 32px;
		font-size: 0.82rem;
	}
	@media (min-width: 720px) {
		.head {
			grid-template-columns: 280px minmax(0, 1fr);
			gap: 2.5rem;
			align-items: end;
		}
		.art {
			width: 100%;
		}
		.info,
		.meta,
		.actions,
		.genres {
			text-align: left;
			justify-content: flex-start;
		}
		.progress {
			margin-left: 0;
		}
		.more {
			margin-left: -0.25rem;
		}
	}
	section {
		max-width: 680px;
		margin-bottom: 2.25rem;
	}
	section h2 {
		padding-top: 1.5rem;
		border-top: 1px solid var(--line);
		margin-bottom: 0.9rem;
	}
	.desc {
		white-space: pre-line;
		font-family: var(--font-display);
		font-size: 1.06rem;
		line-height: 1.65;
	}
	.chapters {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.chapters li + li {
		border-top: 1px solid var(--line);
	}
	.chapters button {
		display: flex;
		gap: 1rem;
		width: 100%;
		min-height: 50px;
		align-items: center;
		border: 0;
		background: transparent;
		text-align: left;
		padding: 0.35rem 0;
	}
	.chapters button:hover .ct {
		text-decoration: underline;
		text-decoration-color: var(--line);
		text-underline-offset: 3px;
	}
	.num {
		width: 1.75rem;
		flex: none;
		color: var(--fg-muted);
		font-size: 0.85rem;
		font-variant-numeric: tabular-nums;
	}
	.ct {
		flex: 1;
	}
	.when {
		font-size: 0.85rem;
		font-variant-numeric: tabular-nums;
	}
	.chapters .current,
	.chapters .current .num {
		color: var(--accent);
	}
	.chapters .current .ct {
		font-family: var(--font-display);
		font-style: italic;
		font-weight: 600;
	}
	.note-edit {
		display: grid;
		gap: 0.5rem;
		width: 100%;
		padding: 0.5rem 0;
	}
	.note-edit textarea {
		width: 100%;
		font: inherit;
		padding: 0.6rem 0.8rem;
		border: 1px solid var(--line);
		border-radius: var(--radius-sm);
		background: var(--bg);
	}
	.note-actions {
		display: flex;
		gap: 0.5rem;
	}
	.marks {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.marks li + li {
		border-top: 1px solid var(--line);
	}
	.mark {
		display: flex;
		flex: 1;
		flex-direction: column;
		align-items: flex-start;
		justify-content: center;
		gap: 0.1rem;
		min-height: 52px;
		border: 0;
		background: transparent;
		text-align: left;
		padding: 0.4rem 0;
	}
	.mark strong {
		font-variant-numeric: tabular-nums;
	}
	.menu {
		display: grid;
		gap: 0.5rem;
		margin-bottom: 1rem;
	}
	.menu .btn {
		justify-content: flex-start;
		border-radius: var(--radius-sm);
	}
	.choice {
		display: flex;
		gap: 0.75rem;
		align-items: center;
		min-height: 48px;
		padding: 0 0.9rem;
		border: 1px solid var(--line);
		border-radius: var(--radius-sm);
		cursor: pointer;
	}
	.choice:has(input:checked) {
		border-color: var(--fg);
	}
</style>
