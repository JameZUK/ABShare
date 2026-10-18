<script lang="ts">
	// Home (SPEC §7.2): the book you're reading, continue listening, the next book in series
	// you're reading, recently added, what's on this device, and genres to browse.
	import { api, session } from '$lib/api/session.svelte';
	import { messageOf } from '$lib/api/http';
	import type { BookDetail, BookSummary, Home, Tile } from '$lib/api/types';
	import { duration, names, timeLeft } from '$lib/format';
	import { downloads } from '$lib/offline/downloads.svelte';
	import { player } from '$lib/player/player.svelte';
	import { span } from '$lib/player/chapters';
	import BookTile from '$lib/components/BookTile.svelte';
	import Cover from '$lib/components/Cover.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import Shelf from '$lib/components/Shelf.svelte';
	import GenreChips from '$lib/components/GenreChips.svelte';

	let home = $state<Home | null>(null);
	let error = $state('');
	let starting = $state(false);

	$effect(() => {
		if (session.status !== 'signed-in') return;
		api<Home>('v1/home')
			.then((h) => (home = h))
			.catch((e) => (error = messageOf(e)));
	});

	const onDevice = $derived(
		downloads.list.map(
			(d): BookSummary => ({
				type: 'book',
				id: d.id,
				title: d.detail.title,
				subtitle: d.detail.subtitle,
				authors: (d.detail.authors ?? []).map((p) => p.name),
				narrators: [],
				series: null,
				duration_ms: d.detail.duration_ms,
				cover: d.detail.cover,
				added_at: d.added,
				progress: null,
			}),
		),
	);

	const firstName = $derived((session.user?.display_name ?? '').trim().split(/\s+/)[0] ?? '');
	const PARTS = ['night', 'morning', 'afternoon', 'evening'];
	const when = (() => {
		const d = new Date();
		const part = PARTS[d.getHours() < 5 ? 0 : d.getHours() < 12 ? 1 : d.getHours() < 18 ? 2 : 3];
		return `${d.toLocaleDateString('en-GB', { weekday: 'long' })} ${part}`;
	})();

	// The book in the player leads; otherwise the most recent one in progress.
	const current = $derived.by((): { id: string; title: string; cover: string | null; authors: string[]; duration: number; position: number } | null => {
		const b = player.book;
		if (b) return { id: b.id, title: b.title, cover: b.cover, authors: (b.authors ?? []).map((p) => p.name), duration: player.duration || b.duration_ms, position: player.position };
		const c = home?.continue[0];
		return c ? { id: c.id, title: c.title, cover: c.cover, authors: c.authors, duration: c.duration_ms, position: c.progress?.position_ms ?? 0 } : null;
	});
	const chapter = $derived.by(() => {
		const b = player.book;
		if (!b || b.id !== current?.id || !b.chapters.length) return '';
		const s = span(b.chapters, player.position, player.duration);
		return s.index >= 0 ? (b.chapters[s.index]?.title ?? '') : '';
	});
	const playingHere = $derived(player.book?.id === current?.id && player.playing);
	const rest = $derived((home?.continue ?? []).filter((b) => b.id !== current?.id));
	const recent = $derived((home?.recently_added ?? []) as Tile[]);

	const recommended = $derived(home?.recommended ?? []);

	async function dismiss(id: number) {
		if (home) home = { ...home, recommended: (home.recommended ?? []).filter((r) => r.id !== id) };
		try {
			await api(`v1/recommendations/${id}/dismiss`, { method: 'POST' });
		} catch (e) {
			error = messageOf(e);
		}
	}

	async function resume() {
		if (!current) return;
		if (player.book?.id === current.id) return player.toggle();
		starting = true;
		try {
			const detail = (await api<BookDetail>(`v1/books/${current.id}`).catch(() => undefined)) ?? (await downloads.detail(current.id));
			if (detail) await player.open(detail, { play: true });
		} catch (e) {
			error = messageOf(e);
		} finally {
			starting = false;
		}
	}
</script>

<main class="page">
	<header class="greet">
		<span class="eyebrow">{when}</span>
		<a class="icon-btn" href="#/library" aria-label="Search the library"><Icon name="search" /></a>
	</header>
	<h1 class="hello">{#if firstName}Welcome back,<br /><em>{firstName}.</em>{:else}Welcome back.{/if}</h1>

	{#if error}<p class="error" role="alert">{error}</p>{/if}

	{#if recommended.length}
		<section class="recs" aria-label="Recommended for you">
			<h2>Recommended for you</h2>
			{#each recommended as r (r.id)}
				<article class="rec">
					<span class="eyebrow accent">Recommended by {r.from}</span>
					{#if r.note}<blockquote class="note serif">{r.note}</blockquote>{/if}
					<div class="rec-books">
						{#each r.books as b (b.id)}<BookTile tile={b} />{/each}
					</div>
					<button class="btn soft notnow" onclick={() => dismiss(r.id)}>Not now</button>
				</article>
			{/each}
		</section>
	{/if}

	{#if current}
		<section class="hero" aria-label="Currently reading">
			<a class="hero-art" href="#/book/{current.id}" aria-label="{current.title}: book details"><Cover id={current.id} src={current.cover} title={current.title} size={320} book /></a>
			<div class="hero-text">
				<span class="eyebrow accent">{playingHere ? 'Now playing' : 'Currently reading'}</span>
				<a class="hero-title serif" href="#/book/{current.id}">{current.title}</a>
				<span class="hero-line serif">{chapter || names(current.authors)}</span>
				<span class="hero-bar" aria-hidden="true"><span style:width="{Math.min(100, (current.position / Math.max(1, current.duration)) * 100)}%"></span></span>
				<div class="hero-foot">
					<span class="muted">{current.position > 0 ? timeLeft(current.duration, current.position) : duration(current.duration)}</span>
					<button class="btn" onclick={resume} disabled={starting}>
						<Icon name={playingHere ? 'pause' : 'play'} size={16} />{playingHere ? 'Pause' : current.position > 0 ? 'Resume' : 'Play'}
					</button>
				</div>
			</div>
		</section>
	{/if}

	{#if home}
		<Shelf title="Continue listening" tiles={rest} />
		<Shelf title="Up next in your series" tiles={home.series_next.map((s) => s.book)} subs={home.series_next.map((s) => (s.book.series?.position ? `${s.series}, ${s.book.series.position}` : s.series))} />

		{#if recent.length}
			<section class="recent">
				<header><h2>Recently added</h2><a href="#/library">See all</a></header>
				<ul class="rules">
					{#each recent.slice(0, 6) as t (t.type + t.id)}
						<li>
							<a href={t.type === 'show' ? `#/group/${t.id}` : `#/book/${t.id}`}>
								<span class="thumb"><Cover id={t.id} src={t.cover} title={t.title} size={128} book /></span>
								<span class="rt">
									<span class="rtitle serif">{t.title}</span>
									<span class="muted">{t.type === 'show' ? `${t.episodes} episodes` : `${names(t.authors)} · ${duration(t.duration_ms)}`}</span>
								</span>
								<Icon name="right" size={18} />
							</a>
						</li>
					{/each}
				</ul>
			</section>
		{/if}

		{#if !home.continue.length && !home.recently_added.length}
			<p class="muted">Nothing here yet. Browse the <a href="#/library">library</a> to start a book.</p>
		{/if}
	{:else if session.status === 'signed-in' && !error}
		<p class="muted">Loading…</p>
	{/if}

	<Shelf title="On this device" tiles={onDevice} more="#/downloads" />

	{#if session.status === 'signed-in'}
		<section class="browse">
			<h2>Browse</h2>
			<GenreChips wrap />
		</section>
	{/if}
</main>

<style>
	.greet {
		display: flex;
		align-items: center;
		justify-content: space-between;
		margin: 0.25rem 0 0.15rem;
	}
	.greet .icon-btn {
		margin-right: -0.6rem;
		color: var(--fg);
	}
	.hello {
		font-weight: 400;
		font-size: clamp(2rem, 7vw, 2.6rem);
		line-height: 1.08;
		margin-bottom: 1.5rem;
	}
	.hello em {
		font-weight: 600;
	}

	.recs {
		margin-bottom: 1.75rem;
		padding-bottom: 0.5rem;
		border-bottom: 1px solid var(--line);
	}
	.recs h2 {
		margin-bottom: 0.9rem;
	}
	.rec + .rec {
		padding-top: 1.25rem;
		border-top: 1px solid var(--line);
	}
	.rec {
		padding-bottom: 1.25rem;
	}
	.note {
		margin: 0.4rem 0 1rem;
		padding: 0;
		font-style: italic;
		font-size: 1.12rem;
		line-height: 1.5;
		white-space: pre-line;
		overflow-wrap: anywhere;
	}
	.note::before {
		content: '“';
	}
	.note::after {
		content: '”';
	}
	.rec-books {
		display: grid;
		grid-auto-flow: column;
		grid-auto-columns: 132px;
		gap: 1.1rem;
		overflow-x: auto;
		scrollbar-width: none;
		margin: 0 calc(-1 * var(--gutter));
		padding: 0.25rem var(--gutter) 1rem;
	}
	.rec-books::-webkit-scrollbar {
		display: none;
	}
	@media (min-width: 1024px) {
		.rec-books {
			grid-auto-columns: 156px;
			gap: 1.5rem;
		}
	}
	.notnow {
		min-height: 38px;
		font-size: 0.875rem;
	}
	.hero {
		display: grid;
		grid-template-columns: 128px minmax(0, 1fr);
		gap: 1.25rem;
		align-items: end;
		padding-bottom: 1.5rem;
		margin-bottom: 1.75rem;
		border-bottom: 1px solid var(--line);
	}
	.hero-art {
		display: block;
	}
	.hero-text {
		display: grid;
		min-width: 0;
	}
	.hero-title {
		color: inherit;
		text-decoration: none;
		font-size: 1.4rem;
		font-weight: 600;
		line-height: 1.15;
		margin-top: 0.35rem;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}
	.hero-line {
		font-style: italic;
		color: var(--fg-muted);
		margin-top: 0.2rem;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.hero-bar {
		display: block;
		height: 2px;
		background: var(--line);
		margin: 0.9rem 0 0.6rem;
	}
	.hero-bar span {
		display: block;
		height: 100%;
		background: var(--fg);
	}
	.hero-foot {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.75rem;
		font-size: 0.875rem;
	}
	.hero-foot .btn {
		min-height: 42px;
		padding: 0 1.1rem;
	}
	@media (min-width: 720px) {
		.hero {
			grid-template-columns: 200px minmax(0, 420px);
			gap: 2rem;
		}
		.hero-title {
			font-size: 1.9rem;
		}
	}

	.recent {
		margin-bottom: 2rem;
	}
	.recent header {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		margin-bottom: 0.4rem;
	}
	.recent h2 {
		margin: 0;
	}
	.recent header a {
		font-size: 0.85rem;
		font-weight: 600;
		text-decoration: none;
	}
	.recent ul {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.recent li a {
		display: flex;
		align-items: center;
		gap: 0.9rem;
		padding: 0.75rem 0;
		color: inherit;
		text-decoration: none;
	}
	.recent li a > :global(svg) {
		color: var(--fg-muted);
		flex: none;
	}
	.thumb {
		width: 48px;
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
	.rtitle {
		font-size: 1.02rem;
		font-weight: 600;
	}
	.browse h2 {
		margin-bottom: 0.9rem;
	}
</style>
