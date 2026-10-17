<script lang="ts">
	// Admins correct a book's details. The corrections are stored by the server and shown
	// instead of the file's tags; "Use the file's details" removes them.
	import { messageOf } from '$lib/api/http';
	import { api, post } from '$lib/api/session.svelte';
	import type { BookDetail } from '$lib/api/types';
	import { names } from '$lib/format';
	import Sheet from './Sheet.svelte';

	let { book, open = $bindable(false), onsaved }: { book: BookDetail; open?: boolean; onsaved: () => void } = $props();
	let title = $state('');
	let subtitle = $state('');
	let authors = $state('');
	let narrators = $state('');
	let series = $state('');
	let position = $state('');
	let genres = $state('');
	let description = $state('');
	let error = $state('');
	let busy = $state(false);

	$effect(() => {
		if (!open) return;
		title = book.title;
		subtitle = book.subtitle ?? '';
		authors = (book.authors ?? []).map((p) => p.name).join(', ');
		narrators = (book.narrators ?? []).map((p) => p.name).join(', ');
		series = book.series?.name ?? '';
		position = book.series?.position ?? '';
		genres = book.genres.join(', ');
		description = book.description ?? '';
		error = '';
	});

	const list = (s: string) => s.split(',').map((x) => x.trim()).filter(Boolean);

	async function save(e: Event) {
		e.preventDefault();
		busy = true;
		error = '';
		try {
			await post(`v1/admin/books/${book.id}/details`, {
				title: title.trim(),
				subtitle: subtitle.trim() || null,
				authors: list(authors),
				narrators: list(narrators),
				series: series.trim() ? { name: series.trim(), position: position.trim() || null } : null,
				genres: list(genres),
				description: description.trim() || null,
			}, 'PUT');
			open = false;
			onsaved();
		} catch (err) {
			error = messageOf(err);
		} finally {
			busy = false;
		}
	}

	async function reset() {
		busy = true;
		try {
			await api(`v1/admin/books/${book.id}/details`, { method: 'DELETE' });
			open = false;
			onsaved();
		} catch (err) {
			error = messageOf(err);
		} finally {
			busy = false;
		}
	}
</script>

<Sheet bind:open title="Edit details">
	<form onsubmit={save}>
		<p class="muted small">Corrections are stored by ABShare; the audiobook files aren’t changed. Separate several names with commas.</p>
		<label class="field">Title <input bind:value={title} required maxlength="300" /></label>
		<label class="field">Subtitle <input bind:value={subtitle} maxlength="300" /></label>
		<label class="field">Authors <input bind:value={authors} maxlength="2000" placeholder={names(book.authors)} /></label>
		<label class="field">Narrators <input bind:value={narrators} maxlength="2000" /></label>
		<div class="pair">
			<label class="field grow">Series <input bind:value={series} maxlength="300" /></label>
			<label class="field num">Book <input bind:value={position} maxlength="10" inputmode="decimal" placeholder="2" /></label>
		</div>
		<label class="field">Genres <input bind:value={genres} maxlength="2000" placeholder="Science Fiction, Space Opera" /></label>
		<label class="field">Description <textarea bind:value={description} rows="5" maxlength="20000"></textarea></label>
		<p class="error" role="alert">{error}</p>
		<div class="actions">
			<button class="btn" disabled={busy}>Save</button>
			<button type="button" class="btn plain" onclick={reset} disabled={busy}>Use the file’s details</button>
		</div>
	</form>
</Sheet>

<style>
	.small {
		font-size: 0.85rem;
	}
	.pair {
		display: flex;
		gap: 0.75rem;
	}
	.grow {
		flex: 1;
	}
	.num {
		width: 6rem;
	}
	textarea {
		padding: 0.6rem 0.8rem;
		border: 1px solid var(--line);
		border-radius: var(--radius-sm);
		background: var(--bg-raised);
		font-weight: 400;
		resize: vertical;
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem;
	}
	form {
		max-height: 70vh;
		overflow-y: auto;
	}
</style>
