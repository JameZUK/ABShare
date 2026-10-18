<script lang="ts">
	// "Where was I?": your own listening of one book, newest first, grouped by day. Each
	// stretch shows when you listened and where in the book it began and ended; tap either
	// place to play from there. Only you can see this (SPEC D31).
	import { api } from '$lib/api/session.svelte';
	import { messageOf } from '$lib/api/http';
	import type { BookDetail } from '$lib/api/types';
	import { clock, duration } from '$lib/format';
	import { player } from '$lib/player/player.svelte';
	import { byDay, timeOfDay } from '$lib/when';
	import Sheet from './Sheet.svelte';

	interface Stretch {
		id: number;
		started_at: number;
		ended_at: number;
		start_position_ms: number;
		end_position_ms: number;
		seconds: number;
	}

	let { book, open = $bindable(false) }: { book: BookDetail; open?: boolean } = $props();
	let stretches = $state<Stretch[] | null>(null);
	let enabled = $state(true);
	let error = $state('');

	$effect(() => {
		if (!open) return;
		stretches = null;
		error = '';
		api<{ enabled: boolean; sessions: Stretch[] }>(`v1/me/history/books/${book.id}`)
			.then((r) => {
				stretches = r.sessions;
				enabled = r.enabled;
			})
			.catch((e) => (error = messageOf(e)));
	});

	const days = $derived(byDay(stretches ?? [], (s) => s.started_at));

	async function playFrom(ms: number) {
		open = false;
		if (player.book?.id === book.id) {
			player.seek(ms);
			if (!player.playing) player.toggle();
		} else {
			await player.open(book, { play: true, at: ms });
		}
		player.expanded = true;
	}
</script>

<Sheet bind:open title="Where was I?">
	{#if error}
		<p class="error" role="alert">{error}</p>
	{:else if stretches === null}
		<p class="muted">Loading…</p>
	{:else if !stretches.length}
		<p class="muted">{enabled ? 'Nothing yet. Each time you listen, it appears here, so you can find your place again.' : 'Listening history is off. Turn it on in Settings → History to keep your places.'}</p>
	{:else}
		<p class="muted intro">Tap a place to play from there.</p>
		{#each days as d (d.day)}
			<h3 class="eyebrow day">{d.day}</h3>
			<ol class="stretches">
				{#each d.items as s (s.id)}
					<li>
						<div class="when">
							<span class="serif time">{timeOfDay(s.started_at)} – {timeOfDay(s.ended_at)}</span>
							<span class="muted">{duration(s.seconds * 1000)}</span>
						</div>
						<div class="places">
							<button class="chip" onclick={() => playFrom(s.start_position_ms)} aria-label="Play from {clock(s.start_position_ms)}, where you started at {timeOfDay(s.started_at)}">
								▶ {clock(s.start_position_ms)}
							</button>
							<span class="arrow" aria-hidden="true">→</span>
							<button class="chip" onclick={() => playFrom(s.end_position_ms)} aria-label="Play from {clock(s.end_position_ms)}, where you stopped at {timeOfDay(s.ended_at)}">
								▶ {clock(s.end_position_ms)}
							</button>
						</div>
					</li>
				{/each}
			</ol>
		{/each}
	{/if}
</Sheet>

<style>
	.intro {
		margin-top: -0.25rem;
		font-size: 0.875rem;
	}
	.day {
		margin: 1.1rem 0 0.25rem;
	}
	.stretches {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.stretches li {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem 1rem;
		padding: 0.7rem 0;
		border-top: 1px solid var(--line);
	}
	.when {
		display: grid;
		font-size: 0.85rem;
	}
	.time {
		font-size: 1.02rem;
		font-weight: 600;
	}
	.places {
		display: flex;
		align-items: center;
		gap: 0.4rem;
	}
	.places .chip {
		min-height: 40px;
		font-variant-numeric: tabular-nums;
	}
	.arrow {
		color: var(--fg-muted);
	}
</style>
