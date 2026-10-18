<script lang="ts">
	// One genre shelf (or all fiction / non-fiction), with the library's search and sort.
	import { page } from '$app/state';
	import { KINDS, isShelfId, loadGenres } from '$lib/api/genres';
	import { session } from '$lib/api/session.svelte';
	import LibraryBrowser from '$lib/components/LibraryBrowser.svelte';

	const id = $derived(page.params.id ?? '');
	const shelf = $derived(isShelfId(id) ? id : null);
	let found = $state<{ id: string; name: string } | null>(null);
	// The umbrella names are known; a shelf's name comes from the server. The effect only
	// reads `id` and the session, never the name it sets (that would loop).
	const name = $derived(KINDS[id] ?? (found?.id === id ? found.name : ''));
	$effect(() => {
		const sid = id;
		if (KINDS[sid] || session.status !== 'signed-in') return;
		loadGenres()
			.then((r) => (found = { id: sid, name: r.genres.find((g) => g.id === sid)?.name ?? 'Genre' }))
			.catch(() => (found = { id: sid, name: 'Genre' }));
	});
</script>

{#key shelf}
	<LibraryBrowser title={name || 'Genre'} {shelf} eyebrow="Genre" back />
{/key}
