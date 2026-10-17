<script lang="ts">
	// A book (SPEC §7.2): cover, title, series, people, length and time left, description and
	// chapters. Play/Resume and Download; the rest is in ⋮.
	import { page } from '$app/state';
	import { api, session } from '$lib/api/session.svelte';
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

	const id = $derived(page.params.id ?? '');
	let book = $state<BookDetail | null>(null);
	let error = $state('');
	let canDownload = $state(true);
	let canSave = $state(false);
	let moreOpen = $state(false);
	let dlOpen = $state(false);
	let marksOpen = $state(false);
	let dlQuality = $state<Quality>('original');
	let dlSize = $state<number | null>(null);

	$effect(() => {
		const bid = id;
		book = null;
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
		else await sync.record(book.id, action);
		book = { ...book, progress: { position_ms: action === 'restart' ? 0 : position, finished: action === 'finished', updated_at: Date.now() } };
		moreOpen = false;
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
	<button class="icon-btn" onclick={() => history.back()} aria-label="Back"><Icon name="back" /></button>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#if book}
		<div class="head">
			<div class="art"><Cover id={book.id} src={book.cover} title={book.title} size={640} /></div>
			<div class="info">
				<h1>{book.title}</h1>
				{#if book.subtitle}<p class="muted">{book.subtitle}</p>{/if}
				{#if book.series && seriesGroup}
					<p><a href="#/group/{seriesGroup.id}">{book.series.position ? `Book ${book.series.position} of ${book.series.name}` : book.series.name}</a></p>
				{/if}
				{#if book.authors?.length}<p>By {names(book.authors)}</p>{/if}
				{#if book.narrators?.length}<p class="muted">Read by {names(book.narrators)}</p>{/if}
				<p class="muted">
					{duration(book.duration_ms)}
					{#if book.progress?.finished}· Finished{:else if position > 0}· {timeLeft(book.duration_ms, position)}{/if}
				</p>
				<div class="actions">
					<button class="btn" onclick={play}>
						<Icon name={isOpen && player.playing ? 'pause' : 'play'} />
						{isOpen && player.playing ? 'Pause' : position > 0 ? `Resume from ${clock(position)}` : 'Play'}
					</button>
					{#if canDownload}
						{#if !dl}
							<button class="btn soft" onclick={openDownload} disabled={session.status !== 'signed-in'}><Icon name="download" /> Download</button>
						{:else if dl.state === 'complete'}
							<span class="btn plain" aria-disabled="true"><Icon name="downloaded" /> Downloaded</span>
						{:else}
							<a class="btn plain" href="#/downloads"><Icon name="download" /> {dl.total ? Math.floor((dl.next / dl.total) * 100) : 0}%</a>
						{/if}
					{/if}
					<button class="icon-btn" onclick={() => (moreOpen = true)} aria-label="More"><Icon name="more" /></button>
				</div>
			</div>
		</div>

		{#if paragraphs.length}
			<section>
				<h2>About this book</h2>
				{#each paragraphs as p, i (i)}<p class="desc">{p}</p>{/each}
			</section>
		{/if}

		{#if book.chapters.length}
			<section>
				<h2>Chapters</h2>
				<ol class="chapters">
					{#each book.chapters as c, i (i)}
						<li>
							<button onclick={() => book && player.open(book, { play: true, at: c.start_ms })}>
								<span class="ct">{c.title}</span><span class="muted">{clock(c.start_ms)}</span>
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
			<button class="btn plain" onclick={() => mark('unfinished')}>Mark not finished</button>
		{:else}
			<button class="btn plain" onclick={() => mark('finished')}>Mark finished</button>
		{/if}
		<button class="btn plain" onclick={() => mark('restart')}>Start again</button>
		<button class="btn plain" onclick={() => { moreOpen = false; marksOpen = true; }}>Bookmarks ({book?.bookmarks.length ?? 0})</button>
		{#if canSave && session.status === 'signed-in'}
			<a class="btn plain" href="v1/media/{id}/file" download onclick={() => (moreOpen = false)}>Save file</a>
		{/if}
		{#if dl}
			<button class="btn plain danger" onclick={() => { void downloads.remove(id); moreOpen = false; }}>Remove download</button>
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

<Sheet bind:open={marksOpen} title="Bookmarks">
	{#if !book?.bookmarks.length}
		<p class="muted">No bookmarks yet. Add one from the player.</p>
	{:else}
		<ul class="marks">
			{#each book.bookmarks as m (m.id)}
				<li class="row">
					<button class="mark" onclick={() => { if (book) void player.open(book, { play: true, at: m.position_ms }); marksOpen = false; }}>
						<strong>{clock(m.position_ms)}</strong>
						{#if m.note}<span class="muted">{m.note}</span>{/if}
					</button>
					<button class="icon-btn" onclick={() => removeMark(m.id)} aria-label="Delete bookmark at {clock(m.position_ms)}"><Icon name="trash" /></button>
				</li>
			{/each}
		</ul>
	{/if}
</Sheet>

<style>
	.head {
		display: grid;
		gap: 1.25rem;
		grid-template-columns: minmax(0, 1fr);
		margin-bottom: 1.5rem;
	}
	@media (min-width: 640px) {
		.head {
			grid-template-columns: 260px minmax(0, 1fr);
		}
	}
	.art {
		max-width: 300px;
		width: 70%;
		margin: 0 auto;
	}
	@media (min-width: 640px) {
		.art {
			width: 100%;
		}
	}
	.info p {
		margin: 0 0 0.35em;
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
		align-items: center;
		margin-top: 1rem;
	}
	.desc {
		white-space: pre-line;
	}
	section {
		margin-bottom: 1.5rem;
	}
	.chapters,
	.marks {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.chapters button,
	.mark {
		display: flex;
		gap: 1rem;
		width: 100%;
		min-height: 44px;
		align-items: center;
		border: 0;
		border-bottom: 1px solid var(--line);
		background: transparent;
		text-align: left;
		padding: 0.25rem 0;
	}
	.mark {
		flex-direction: column;
		align-items: flex-start;
		justify-content: center;
		border: 0;
	}
	.ct {
		flex: 1;
	}
	.menu {
		display: grid;
		gap: 0.5rem;
		margin-bottom: 1rem;
	}
	.choice {
		display: flex;
		gap: 0.6rem;
		align-items: center;
		min-height: 44px;
	}
	.danger {
		color: var(--danger);
	}
</style>
