<script lang="ts">
	// "How was it?" (SPEC D32): when a book is finished on this device, ask once for a rating
	// and an optional short comment. Never twice for the same book for the same person, never
	// when they've already reviewed it, and not while offline.
	import { noteReviewChanged } from '$lib/reviewsChanged.svelte';
	import { messageOf } from '$lib/api/http';
	import { api, post, session } from '$lib/api/session.svelte';
	import type { ReviewsView } from '$lib/api/types';
	import { player } from '$lib/player/player.svelte';
	import { askedBefore, markAsked, shouldPrompt } from '$lib/reviewAsked';
	import { REVIEW_CHARS, REVIEW_WORDS, countChars, countWords } from '$lib/words';
	import Cover from './Cover.svelte';
	import Sheet from './Sheet.svelte';
	import Stars from './Stars.svelte';

	let book = $state<{ id: string; title: string; cover: string | null } | null>(null);
	let open = $state(false);
	let rating = $state(0);
	let comment = $state('');
	let busy = $state(false);
	let error = $state('');
	let thanks = $state(false);

	const words = $derived(countWords(comment));
	const tooLong = $derived(words > REVIEW_WORDS || countChars(comment) > REVIEW_CHARS);
	const canSave = $derived(rating >= 1 && rating <= 5 && !tooLong && !busy);

	$effect(() => {
		const done = player.finishedNow;
		if (!done) return;
		player.finishedNow = null;
		void consider(done);
	});

	async function consider(done: { id: string; title: string; cover: string | null }) {
		const user = session.user?.username;
		const online = session.status === 'signed-in';
		if (!user || !online || askedBefore(user, done.id)) return;
		let reviewed = false;
		try {
			reviewed = (await api<ReviewsView>(`v1/books/${done.id}/reviews`)).mine !== null;
		} catch {
			return; // can't tell: don't ask now
		}
		if (!shouldPrompt({ online, asked: false, reviewed })) {
			if (reviewed) markAsked(user, done.id);
			return;
		}
		markAsked(user, done.id);
		book = done;
		rating = 0;
		comment = '';
		error = '';
		thanks = false;
		open = true;
	}

	async function save() {
		if (!book || !canSave) return;
		busy = true;
		error = '';
		try {
			await post(`v1/books/${book.id}/review`, { rating, comment: comment.trim() }, 'PUT');
			noteReviewChanged(book.id);
			thanks = true;
			setTimeout(() => (open = false), 1200);
		} catch (e) {
			error = messageOf(e);
		} finally {
			busy = false;
		}
	}
</script>

<Sheet bind:open title="How was it?">
	{#if book}
		<div class="head">
			<span class="art"><Cover id={book.id} src={book.cover} title={book.title} size={128} book /></span>
			<p class="serif q">You finished <em>{book.title}</em>. How was it?</p>
		</div>
		{#if thanks}
			<p class="thanks" role="status">Thank you. Your review is on the book’s page.</p>
		{:else}
			<form onsubmit={(e) => { e.preventDefault(); void save(); }}>
				<Stars input bind:value={rating} size={32} name="prompt-rating" label="Your rating" />
				<label class="field">
					<span class="visually-hidden">Your comment (optional)</span>
					<textarea bind:value={comment} rows="3" maxlength="2000" placeholder="A few words for others (optional, up to {REVIEW_WORDS} words)"></textarea>
				</label>
				<p class="count" class:over={tooLong} aria-live="polite">{words} / {REVIEW_WORDS} words</p>
				{#if error}<p class="error" role="alert">{error}</p>{/if}
				<div class="actions">
					<button type="button" class="btn soft" onclick={() => (open = false)}>Not now</button>
					<button type="submit" class="btn" disabled={!canSave}>Save</button>
				</div>
			</form>
		{/if}
	{/if}
</Sheet>

<style>
	.head {
		display: flex;
		align-items: center;
		gap: 1rem;
		margin-bottom: 1rem;
	}
	.art {
		width: 64px;
		flex: none;
	}
	.q {
		margin: 0;
		font-size: 1.1rem;
		line-height: 1.35;
	}
	.count {
		margin: -0.6rem 0 0.75rem;
		font-size: 0.8rem;
		color: var(--fg-muted);
	}
	.count.over {
		color: var(--danger);
		font-weight: 600;
	}
	.actions {
		display: flex;
		justify-content: flex-end;
		gap: 0.6rem;
	}
	.thanks {
		font-family: var(--font-display);
		font-style: italic;
		margin: 0.5rem 0 1rem;
	}
</style>
