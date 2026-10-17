<script lang="ts">
	// The full-screen player (SPEC §7.2): chapter-relative scrubber with whole-book progress
	// underneath; skip, play/pause and chapter buttons; speed, sleep, chapters and bookmarks.
	import { post } from '$lib/api/session.svelte';
	import { messageOf } from '$lib/api/http';
	import { clock, timeLeft } from '$lib/format';
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
		<header class="row">
			<button class="icon-btn" onclick={() => (player.expanded = false)} aria-label="Close the player"><Icon name="chevron" /></button>
			<span class="spacer"></span>
			<a class="icon-btn" href="#/book/{b.id}" onclick={() => (player.expanded = false)} aria-label="Book details"><Icon name="info" /></a>
			<button class="icon-btn" onclick={() => (moreOpen = true)} aria-label="More"><Icon name="more" /></button>
		</header>

		<div class="art"><Cover id={b.id} src={b.cover} title={b.title} size={640} /></div>

		<div class="titles">
			<h1>{b.title}</h1>
			<p class="muted">{s.index >= 0 ? b.chapters[s.index]?.title : (b.authors ?? []).map((p) => p.name).join(', ')}</p>
		</div>

		<p class="status" role="status">{toast ?? player.notice ?? (player.buffering ? 'Loading…' : '')}</p>

		<div class="scrub">
			<input
				type="range"
				min="0"
				max="1000"
				step="1"
				value={Math.round(frac * 1000)}
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
			<div class="times muted">
				<span>{clock(shown - s.start)}</span>
				<span>−{clock(s.end - shown)}</span>
			</div>
			<div class="book" aria-hidden="true"><span style:width="{bookFrac * 100}%"></span></div>
			<p class="muted small">{timeLeft(player.duration, shown)} in the book{player.speed !== 1 ? ` at ${player.speed}×` : ''}</p>
		</div>

		<div class="controls">
			<button class="icon-btn" onclick={() => player.previousChapter()} aria-label="Previous chapter" disabled={b.chapters.length === 0}><Icon name="prev" /></button>
			<button class="icon-btn skip" onclick={() => player.skip(-settings.prefs.skipBack)} aria-label="Back {settings.prefs.skipBack} seconds">
				<Icon name="back15" size={30} /><span>{settings.prefs.skipBack}</span>
			</button>
			<button class="big" onclick={() => player.toggle()} aria-label={player.playing ? 'Pause' : 'Play'} disabled={player.preparing}>
				<Icon name={player.playing ? 'pause' : 'play'} size={36} />
			</button>
			<button class="icon-btn skip" onclick={() => player.skip(settings.prefs.skipForward)} aria-label="Forward {settings.prefs.skipForward} seconds">
				<Icon name="fwd30" size={30} /><span>{settings.prefs.skipForward}</span>
			</button>
			<button class="icon-btn" onclick={() => player.nextChapter()} aria-label="Next chapter" disabled={b.chapters.length === 0}><Icon name="next" /></button>
		</div>

		<div class="tools">
			<button class="tool" onclick={() => (speedOpen = true)}><Icon name="speed" /><span>{player.speed}×</span></button>
			<button class="tool" onclick={() => (sleepOpen = true)} aria-pressed={player.sleep.kind !== 'off'}><Icon name="moon" /><span>{sleepLabel(player.sleep)}</span></button>
			<button class="tool" onclick={() => (chaptersOpen = true)} disabled={b.chapters.length === 0}><Icon name="list" /><span>Chapters</span></button>
			<button class="tool" onclick={addBookmark}><Icon name="bookmark" /><span>Bookmark</span></button>
		</div>
		</div>
	</div>

	<Sheet bind:open={speedOpen} title="Speed">
		<div class="choices">
			{#each SPEEDS as sp (sp)}
				<button class="btn" class:plain={player.speed !== sp} onclick={() => { player.setSpeed(sp); speedOpen = false; }}>{sp}×</button>
			{/each}
		</div>
	</Sheet>

	<Sheet bind:open={sleepOpen} title="Sleep timer">
		<div class="choices">
			{#each [5, 10, 15, 30, 45, 60] as m (m)}
				<button class="btn plain" onclick={() => { player.setSleep({ kind: 'time', remainingMs: m * 60_000 }); sleepOpen = false; }}>{m} min</button>
			{/each}
			<button class="btn plain" disabled={b.chapters.length === 0} onclick={() => { player.setSleep({ kind: 'chapter' }); sleepOpen = false; }}>End of chapter</button>
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
		<p class="muted">Streaming quality</p>
		<div class="choices">
			{#if player.source === 'device'}
				<p>Playing from this device.</p>
			{:else}
				<button class="btn" class:plain={player.source !== 'original'} onclick={() => { void player.setSource('original'); moreOpen = false; }}>Original</button>
				{#each KBPS as k (k)}
					<button class="btn" class:plain={player.source !== k} onclick={() => { void player.setSource(k); moreOpen = false; }}>Data saver {k} kb/s</button>
				{/each}
			{/if}
		</div>
		<p class="muted">Book</p>
		<div class="choices">
			<button class="btn plain" onclick={() => { void player.mark('finished'); moreOpen = false; }}>Mark finished</button>
			<button class="btn plain" onclick={() => { void player.mark('restart'); moreOpen = false; }}>Start again</button>
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
	}
	.col {
		display: grid;
		grid-template-rows: auto minmax(0, 1fr) auto auto auto auto auto;
		height: 100%;
		min-height: 520px;
		max-width: 560px;
		margin: 0 auto;
		padding: env(safe-area-inset-top, 0) 1rem calc(1rem + var(--safe-b));
	}
	.art {
		display: flex;
		justify-content: center;
		min-height: 0;
		padding: 0.5rem 0;
	}
	.art :global(.cover) {
		height: 100%;
		max-height: 46vh;
		width: auto;
		max-width: 100%;
		aspect-ratio: 1;
		box-shadow: var(--shadow);
	}
	.titles {
		text-align: center;
	}
	.titles h1 {
		font-size: 1.3rem;
		margin: 0.25rem 0;
	}
	.status {
		min-height: 1.4em;
		text-align: center;
		margin: 0;
		font-size: 0.9rem;
		color: var(--fg-muted);
	}
	.scrub input {
		width: 100%;
		min-height: 44px;
		accent-color: var(--accent);
	}
	.times {
		display: flex;
		justify-content: space-between;
		font-size: 0.85rem;
		font-variant-numeric: tabular-nums;
	}
	.book {
		height: 3px;
		background: var(--bg-sunken);
		border-radius: 2px;
		margin-top: 0.5rem;
	}
	.book span {
		display: block;
		height: 100%;
		background: var(--fg-muted);
		border-radius: 2px;
	}
	.small {
		font-size: 0.8rem;
		text-align: center;
		margin: 0.3rem 0 0;
	}
	.controls {
		display: flex;
		justify-content: space-around;
		align-items: center;
		margin: 0.75rem 0;
	}
	.skip {
		position: relative;
		width: 56px;
		height: 56px;
	}
	.skip span {
		position: absolute;
		font-size: 0.65rem;
		font-weight: 700;
		top: 50%;
		left: 50%;
		transform: translate(-50%, -38%);
	}
	.big {
		width: 76px;
		height: 76px;
		border-radius: 50%;
		border: 0;
		background: var(--accent);
		color: var(--accent-fg);
		display: grid;
		place-items: center;
	}
	.tools {
		display: grid;
		grid-template-columns: repeat(4, 1fr);
		gap: 0.25rem;
	}
	.tool {
		display: grid;
		justify-items: center;
		gap: 0.15rem;
		min-height: 56px;
		border: 0;
		background: transparent;
		font-size: 0.75rem;
		border-radius: var(--radius-sm);
	}
	.tool[aria-pressed='true'] {
		color: var(--accent);
	}
	.choices {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
		margin-bottom: 1rem;
	}
	.chapters {
		list-style: none;
		margin: 0;
		padding: 0;
		max-height: 60vh;
		overflow-y: auto;
	}
	.chapters button {
		display: flex;
		width: 100%;
		gap: 1rem;
		min-height: 44px;
		align-items: center;
		border: 0;
		background: transparent;
		text-align: left;
		padding: 0.25rem 0.5rem;
		border-radius: var(--radius-sm);
	}
	.chapters button.current {
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
		font-weight: 600;
	}
	.ct {
		flex: 1;
	}
</style>
