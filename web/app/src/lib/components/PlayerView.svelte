<script lang="ts">
	// The full-screen player (SPEC §7.2): chapter-relative scrubber with whole-book progress
	// underneath; skip, play/pause and chapter buttons; speed, sleep, chapters and bookmarks.
	import { post } from '$lib/api/session.svelte';
	import { messageOf } from '$lib/api/http';
	import { clock, names, timeLeft } from '$lib/format';
	import { span } from '$lib/player/chapters';
	import { player, type Sleep } from '$lib/player/player.svelte';
	import { KBPS, SPEEDS, settings } from '$lib/settings.svelte';
	import Cover from './Cover.svelte';
	import Icon from './Icon.svelte';
	import Sheet from './Sheet.svelte';

	const b = $derived(player.book);
	const s = $derived(b ? span(b.chapters, player.position, player.duration) : { start: 0, end: 1, index: -1 });
	// While dragging the scrubber only previews; the seek happens on release (one seek per
	// action: Firefox on Android stalls on a storm of seeks).
	let dragging = $state(false);
	let preview = $state(0);
	const shown = $derived(dragging ? preview : player.position);
	const frac = $derived(Math.min(1, Math.max(0, (shown - s.start) / Math.max(1, s.end - s.start))));
	const people = $derived(b ? [names(b.authors ?? []), b.narrators?.length ? `read by ${names(b.narrators)}` : ''].filter(Boolean).join(' · ') : '');
	const bookFrac = $derived(Math.min(1, shown / Math.max(1, player.duration)));

	let speedOpen = $state(false);
	let sleepOpen = $state(false);
	let chaptersOpen = $state(false);
	let moreOpen = $state(false);
	let toast = $state<string | null>(null);

	function sleepLabel(sl: Sleep): string {
		if (sl.kind === 'time') return `${Math.ceil(sl.remainingMs / 60_000)} min`;
		if (sl.kind === 'chapter') return 'End of chapter';
		return 'Sleep';
	}

	async function addBookmark() {
		if (!b) return;
		try {
			await post(`v1/books/${b.id}/bookmarks`, { position_ms: Math.round(player.position) });
			toast = `Bookmarked at ${clock(player.position)}`;
		} catch (e) {
			toast = messageOf(e);
		}
		setTimeout(() => (toast = null), 3000);
	}

	function onKey(e: KeyboardEvent) {
		if (e.key === 'Escape' && !speedOpen && !sleepOpen && !chaptersOpen && !moreOpen) player.expanded = false;
	}
</script>

<svelte:window onkeydown={onKey} />

{#if b}
	<div class="player" role="dialog" aria-modal="true" aria-label="Player">
		<div class="col">
			<header class="top">
				<button class="icon-btn" onclick={() => (player.expanded = false)} aria-label="Close the player"><Icon name="chevron" size={26} /></button>
				<span class="eyebrow">{player.source === 'device' ? 'On this device' : player.source === 'original' ? 'Now reading' : `Data saver · ${player.source} kb/s`}</span>
				<span class="top-end">
					<a class="icon-btn" href="#/book/{b.id}" onclick={() => (player.expanded = false)} aria-label="Book details"><Icon name="info" /></a>
					<button class="icon-btn" onclick={() => (moreOpen = true)} aria-label="More"><Icon name="more" /></button>
				</span>
			</header>

			<div class="art"><span class="book"><Cover id={b.id} src={b.cover} title={b.title} size={640} book /></span></div>

			<div class="titles">
				<h1>{b.title}</h1>
				<p class="people">{people}</p>
				{#if s.index >= 0}<p class="chapter serif">{b.chapters[s.index]?.title}</p>{/if}
			</div>

			<p class="status" role="status">{toast ?? player.notice ?? (player.buffering ? 'Loading…' : '')}</p>

			<div class="scrub">
				<input
					type="range"
					min="0"
					max="1000"
					step="1"
					value={Math.round(frac * 1000)}
					style:--p="{frac * 100}%"
					aria-label={s.index >= 0 ? 'Position in chapter' : 'Position in book'}
					aria-valuetext={clock(shown)}
					oninput={(e) => {
						dragging = true;
						preview = s.start + (Number(e.currentTarget.value) / 1000) * (s.end - s.start);
					}}
					onchange={(e) => {
						const t = s.start + (Number(e.currentTarget.value) / 1000) * (s.end - s.start);
						dragging = false;
						player.seek(t);
					}}
				/>
				<div class="times">
					<span>{clock(shown - s.start)}</span>
					<span class="left">{timeLeft(player.duration, shown)} in the book{player.speed !== 1 ? ` at ${player.speed}×` : ''}</span>
					<span>−{clock(s.end - shown)}</span>
				</div>
				{#if s.index >= 0}<div class="book-bar" aria-hidden="true"><span style:width="{bookFrac * 100}%"></span></div>{/if}
			</div>

			<div class="controls">
				<button class="icon-btn" onclick={() => player.previousChapter()} aria-label="Previous chapter" disabled={b.chapters.length === 0}><Icon name="prev" /></button>
				<button class="icon-btn skip" onclick={() => player.skip(-settings.prefs.skipBack)} aria-label="Back {settings.prefs.skipBack} seconds">
					<Icon name="back15" size={34} /><span>{settings.prefs.skipBack}</span>
				</button>
				<button class="big" onclick={() => player.toggle()} aria-label={player.playing ? 'Pause' : 'Play'} disabled={player.preparing}>
					<Icon name={player.playing ? 'pause' : 'play'} size={32} />
				</button>
				<button class="icon-btn skip" onclick={() => player.skip(settings.prefs.skipForward)} aria-label="Forward {settings.prefs.skipForward} seconds">
					<Icon name="fwd30" size={34} /><span>{settings.prefs.skipForward}</span>
				</button>
				<button class="icon-btn" onclick={() => player.nextChapter()} aria-label="Next chapter" disabled={b.chapters.length === 0}><Icon name="next" /></button>
			</div>

			<div class="tools">
				<button class="tool" onclick={() => (speedOpen = true)}><span class="speed serif">{player.speed}×</span><span>Speed</span></button>
				<button class="tool" onclick={() => (sleepOpen = true)} aria-pressed={player.sleep.kind !== 'off'}><Icon name="moon" /><span>{sleepLabel(player.sleep)}</span></button>
				<button class="tool" onclick={() => (chaptersOpen = true)} disabled={b.chapters.length === 0}><Icon name="list" /><span>Chapters</span></button>
				<button class="tool" onclick={addBookmark}><Icon name="bookmark" /><span>Bookmark</span></button>
			</div>
		</div>
	</div>

	<Sheet bind:open={speedOpen} title="Speed">
		<div class="choices">
			{#each SPEEDS as sp (sp)}
				<button class="chip" aria-pressed={player.speed === sp} onclick={() => { player.setSpeed(sp); speedOpen = false; }}>{sp}×</button>
			{/each}
		</div>
	</Sheet>

	<Sheet bind:open={sleepOpen} title="Sleep timer">
		<div class="choices">
			{#each [5, 10, 15, 30, 45, 60] as m (m)}
				<button class="chip" onclick={() => { player.setSleep({ kind: 'time', remainingMs: m * 60_000 }); sleepOpen = false; }}>{m} min</button>
			{/each}
			<button class="chip" disabled={b.chapters.length === 0} aria-pressed={player.sleep.kind === 'chapter'} onclick={() => { player.setSleep({ kind: 'chapter' }); sleepOpen = false; }}>End of chapter</button>
			{#if player.sleep.kind !== 'off'}
				<button class="btn soft" onclick={() => { player.setSleep({ kind: 'off' }); sleepOpen = false; }}>Turn off</button>
			{/if}
		</div>
	</Sheet>

	<Sheet bind:open={chaptersOpen} title="Chapters">
		<ol class="chapters">
			{#each b.chapters as c, i (i)}
				<li>
					<button class:current={i === s.index} onclick={() => { player.seek(c.start_ms); chaptersOpen = false; }} aria-current={i === s.index ? 'true' : undefined}>
						<span class="ct">{c.title}</span>
						<span class="muted">{clock(c.start_ms)}</span>
					</button>
				</li>
			{/each}
		</ol>
	</Sheet>

	<Sheet bind:open={moreOpen} title="More">
		<p class="eyebrow">Streaming quality</p>
		<div class="choices">
			{#if player.source === 'device'}
				<p>Playing from this device.</p>
			{:else}
				<button class="chip" aria-pressed={player.source === 'original'} onclick={() => { void player.setSource('original'); moreOpen = false; }}>Original</button>
				{#each KBPS as k (k)}
					<button class="chip" aria-pressed={player.source === k} onclick={() => { void player.setSource(k); moreOpen = false; }}>Data saver {k} kb/s</button>
				{/each}
			{/if}
		</div>
		<p class="eyebrow">Book</p>
		<div class="choices">
			<button class="btn soft" onclick={() => { void player.mark('finished'); moreOpen = false; }}>Mark finished</button>
			<button class="btn soft" onclick={() => { void player.mark('restart'); moreOpen = false; }}>Start again</button>
		</div>
	</Sheet>
{/if}

<style>
	.player {
		position: fixed;
		inset: 0;
		z-index: 40;
		background: var(--bg);
		overflow-y: auto;
		animation: slide 0.32s var(--ease);
	}
	@keyframes slide {
		from {
			transform: translateY(24px);
			opacity: 0;
		}
	}
	.col {
		display: grid;
		grid-template-rows: auto minmax(0, 1fr) auto auto auto auto auto;
		height: 100%;
		min-height: 600px;
		max-width: 480px;
		margin: 0 auto;
		padding: env(safe-area-inset-top, 0) 1.5rem calc(1rem + var(--safe-b));
	}
	.top {
		display: grid;
		grid-template-columns: 1fr auto 1fr;
		align-items: center;
		margin: 0.25rem -0.6rem 0;
	}
	.top-end {
		display: flex;
		justify-content: flex-end;
	}
	.art {
		display: flex;
		justify-content: center;
		align-items: center;
		min-height: 0;
		padding: 1rem 0 0.5rem;
	}
	.book {
		display: block;
		height: 100%;
		max-height: min(42vh, 340px);
		aspect-ratio: 1;
		max-width: 100%;
	}
	.book :global(.frame) {
		height: 100%;
	}
	.titles {
		text-align: center;
		margin-top: 1rem;
	}
	.titles h1 {
		font-size: 1.6rem;
		margin: 0;
	}
	.people {
		margin: 0.3rem 0 0;
		font-size: 0.9rem;
		color: var(--fg-muted);
	}
	.chapter {
		margin: 0.6rem 0 0;
		font-style: italic;
		color: var(--accent);
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.status {
		min-height: 1.4em;
		text-align: center;
		margin: 0.25rem 0 0;
		font-size: 0.85rem;
		color: var(--fg-muted);
	}
	/* The scrubber: a hairline that fills with ink, and a red thumb ringed in the page colour. */
	.scrub input {
		-webkit-appearance: none;
		appearance: none;
		display: block;
		width: 100%;
		height: 32px;
		margin: 0;
		background: transparent;
		cursor: pointer;
		--track: linear-gradient(var(--fg), var(--fg)) 0 50% / var(--p) 2px no-repeat, linear-gradient(var(--line), var(--line)) 0 50% / 100% 2px no-repeat;
	}
	.scrub input::-webkit-slider-runnable-track {
		height: 32px;
		background: var(--track);
	}
	.scrub input::-moz-range-track {
		height: 32px;
		background: var(--track);
	}
	.scrub input::-webkit-slider-thumb {
		-webkit-appearance: none;
		width: 18px;
		height: 18px;
		margin-top: 7px;
		border-radius: 50%;
		background: var(--accent);
		border: 0;
		box-shadow: 0 0 0 4px var(--bg);
	}
	.scrub input::-moz-range-thumb {
		width: 18px;
		height: 18px;
		border-radius: 50%;
		background: var(--accent);
		border: 0;
		box-shadow: 0 0 0 4px var(--bg);
	}
	.scrub input:focus-visible {
		outline-offset: 0;
	}
	.times {
		display: flex;
		justify-content: space-between;
		gap: 0.5rem;
		font-size: 0.78rem;
		color: var(--fg-muted);
		font-variant-numeric: tabular-nums;
	}
	.times .left {
		text-align: center;
	}
	.book-bar {
		height: 2px;
		margin-top: 0.6rem;
		background: var(--line);
		opacity: 0.8;
	}
	.book-bar span {
		display: block;
		height: 100%;
		background: var(--fg-muted);
	}
	.controls {
		display: flex;
		justify-content: space-between;
		align-items: center;
		margin: 1.1rem 0 1.25rem;
	}
	.skip {
		position: relative;
		width: 58px;
		height: 58px;
	}
	.skip span {
		position: absolute;
		font-size: 0.66rem;
		font-weight: 600;
		top: 50%;
		left: 50%;
		transform: translate(-50%, -40%);
	}
	.big {
		width: 78px;
		height: 78px;
		border-radius: 50%;
		border: 0;
		background: var(--accent);
		color: var(--accent-fg);
		display: grid;
		place-items: center;
		box-shadow: 0 12px 24px -10px var(--accent);
		transition: transform 0.15s var(--ease);
	}
	.big:active {
		transform: scale(0.95);
	}
	.tools {
		display: grid;
		grid-template-columns: repeat(4, 1fr);
		gap: 0.25rem;
		padding-top: 0.75rem;
		border-top: 1px solid var(--line);
	}
	.tool {
		display: grid;
		justify-items: center;
		align-content: center;
		gap: 0.2rem;
		min-height: 58px;
		border: 0;
		background: transparent;
		font-size: 0.75rem;
		color: var(--fg-muted);
		border-radius: var(--radius-sm);
	}
	.tool :global(svg),
	.speed {
		color: var(--fg);
	}
	.speed {
		font-size: 1.1rem;
		font-weight: 600;
		line-height: 22px;
	}
	.tool[aria-pressed='true'],
	.tool[aria-pressed='true'] :global(svg) {
		color: var(--accent);
	}
	.tool:disabled {
		opacity: 0.4;
	}
	.choices {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
		margin-bottom: 1.25rem;
	}
	.choices .chip {
		min-height: 44px;
		padding: 0 1.1rem;
	}
	.chapters {
		list-style: none;
		margin: 0;
		padding: 0;
		max-height: 60vh;
		overflow-y: auto;
	}
	.chapters li + li {
		border-top: 1px solid var(--line);
	}
	.chapters button {
		display: flex;
		width: 100%;
		gap: 1rem;
		min-height: 48px;
		align-items: center;
		border: 0;
		background: transparent;
		text-align: left;
		padding: 0.25rem 0.25rem;
	}
	.chapters button.current {
		color: var(--accent);
		font-weight: 600;
	}
	.chapters button.current .ct {
		font-family: var(--font-display);
		font-style: italic;
	}
	.ct {
		flex: 1;
	}
	.eyebrow {
		margin: 0 0 0.6rem;
	}
</style>
