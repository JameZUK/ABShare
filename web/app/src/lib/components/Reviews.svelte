<script lang="ts">
	// A book's reviews: your own (stars and up to 50 words), then everyone's. Names and comments
	// come from other people, so they are shown only as text. Admins can remove someone's
	// review (with a step-up).
	import { reviewsChanged } from '$lib/reviewsChanged.svelte';
	import { adminApi } from '$lib/api/admin';
	import { messageOf } from '$lib/api/http';
	import { api, post, session } from '$lib/api/session.svelte';
	import type { ReviewsView } from '$lib/api/types';
	import { withStepUp } from '$lib/stepup.svelte';
	import { REVIEW_CHARS, REVIEW_WORDS, countChars, countWords } from '$lib/words';
	import Stars from './Stars.svelte';

	let { bookId, onsummary }: { bookId: string; onsummary?: (s: { average: number | null; count: number }) => void } = $props();

	let view = $state<ReviewsView | null>(null);
	let rating = $state(0);
	let comment = $state('');
	let editing = $state(false);
	let busy = $state(false);
	let error = $state('');

	const words = $derived(countWords(comment));
	const tooLong = $derived(words > REVIEW_WORDS || countChars(comment) > REVIEW_CHARS);
	const canSave = $derived(rating >= 1 && rating <= 5 && !tooLong && !busy);
	const others = $derived(view?.reviews ?? []);
	const isAdmin = $derived(session.user?.role === 'admin');

	async function load() {
		try {
			view = await api<ReviewsView>(`v1/books/${bookId}/reviews`);
			rating = view.mine?.rating ?? 0;
			comment = view.mine?.comment ?? '';
			editing = !view.mine;
			onsummary?.({ average: view.average, count: view.count });
		} catch (e) {
			error = messageOf(e);
		}
	}

	$effect(() => {
		void bookId;
		// Reload when this book's reviews change elsewhere (the "How was it?" prompt).
		if (reviewsChanged.book === bookId) void reviewsChanged.n;
		if (session.status === 'signed-in') void load();
	});

	async function save() {
		if (!canSave) return;
		busy = true;
		error = '';
		try {
			await post(`v1/books/${bookId}/review`, { rating, comment: comment.trim() }, 'PUT');
			await load();
		} catch (e) {
			error = messageOf(e);
		} finally {
			busy = false;
		}
	}

	async function remove() {
		busy = true;
		error = '';
		try {
			await api(`v1/books/${bookId}/review`, { method: 'DELETE' });
			await load();
		} catch (e) {
			error = messageOf(e);
		} finally {
			busy = false;
		}
	}

	async function removeOther(userId: number) {
		error = '';
		try {
			await withStepUp(() => adminApi.removeReview(bookId, userId));
			await load();
		} catch (e) {
			error = messageOf(e);
		}
	}

	const when = (at: number) => new Date(at * 1000).toLocaleDateString('en-GB', { day: 'numeric', month: 'short', year: 'numeric' });
</script>

<section class="reviews" id="reviews">
	<h2>Reviews</h2>
	{#if error}<p class="error" role="alert">{error}</p>{/if}

	{#if session.status === 'signed-in' && view}
		{#if view.mine && !editing}
			<div class="mine">
				<span class="eyebrow">Your review</span>
				<Stars value={view.mine.rating} />
				{#if view.mine.comment}<p class="text">{view.mine.comment}</p>{/if}
				<div class="row">
					<button class="btn soft" onclick={() => (editing = true)}>Edit</button>
					<button class="btn danger" onclick={remove} disabled={busy}>Delete</button>
				</div>
			</div>
		{:else}
			<form class="mine" onsubmit={(e) => { e.preventDefault(); void save(); }}>
				<span class="eyebrow">{view.mine ? 'Edit your review' : 'Your review'}</span>
				<Stars input bind:value={rating} size={28} name="rating-{bookId}" label="Your rating" />
				<label class="field">
					<span class="visually-hidden">Your comment (optional)</span>
					<textarea bind:value={comment} rows="3" maxlength="2000" placeholder="What did you think? (optional, up to {REVIEW_WORDS} words)"></textarea>
				</label>
				<div class="row">
					<span class="count" class:over={tooLong} aria-live="polite">{words} / {REVIEW_WORDS} words</span>
					<span class="spacer"></span>
					{#if view.mine}<button type="button" class="btn soft" onclick={() => { editing = false; rating = view?.mine?.rating ?? 0; comment = view?.mine?.comment ?? ''; }}>Cancel</button>{/if}
					<button type="submit" class="btn" disabled={!canSave}>Save</button>
				</div>
			</form>
		{/if}
	{/if}

	{#if others.length}
		<ul class="list">
			{#each others as r (r.mine ? 'me' : `${r.user_id ?? ''}${r.name}${r.updated_at}`)}
				<li>
					<div class="head">
						<span class="name serif">{r.mine ? `${r.name} (you)` : r.name}</span>
						<Stars value={r.rating} size={15} />
						<span class="spacer"></span>
						<span class="muted date">{when(r.updated_at)}</span>
						{#if isAdmin && !r.mine && r.user_id !== undefined}
							<button class="btn danger small" onclick={() => removeOther(r.user_id!)} aria-label="Remove {r.name}’s review">Remove</button>
						{/if}
					</div>
					{#if r.comment}<p class="text">{r.comment}</p>{/if}
				</li>
			{/each}
		</ul>
	{:else if view}
		<p class="muted">No reviews yet{view.mine ? '' : '. Be the first'}.</p>
	{/if}
</section>

<style>
	/* Matches the book page's other sections. */
	.reviews {
		max-width: 680px;
		margin-bottom: 2.25rem;
	}
	h2 {
		padding-top: 1.5rem;
		border-top: 1px solid var(--line);
		margin-bottom: 0.9rem;
	}
	.mine {
		display: grid;
		gap: 0.5rem;
		padding-bottom: 1.25rem;
		margin-bottom: 0.5rem;
		border-bottom: 1px solid var(--line);
	}
	.mine .field {
		margin: 0;
	}
	.mine textarea {
		width: 100%;
	}
	.count {
		font-size: 0.85rem;
		color: var(--fg-muted);
		font-variant-numeric: tabular-nums;
	}
	.count.over {
		color: var(--danger);
		font-weight: 600;
	}
	.list {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.list li {
		padding: 0.9rem 0;
	}
	.list li + li {
		border-top: 1px solid var(--line);
	}
	.head {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		flex-wrap: wrap;
	}
	.name {
		font-weight: 600;
		font-size: 1.02rem;
	}
	.date {
		font-size: 0.82rem;
	}
	.text {
		margin: 0.35rem 0 0;
		white-space: pre-line;
		overflow-wrap: anywhere;
	}
	.small {
		min-height: 32px;
		padding: 0 0.8rem;
		font-size: 0.82rem;
	}
</style>
