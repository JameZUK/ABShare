<script lang="ts">
	// Home (SPEC §7.2): continue listening, the next book in series you're reading, recently
	// added, and what's on this device.
	import { api, session } from '$lib/api/session.svelte';
	import { messageOf } from '$lib/api/http';
	import type { BookSummary, Home } from '$lib/api/types';
	import { downloads } from '$lib/offline/downloads.svelte';
	import Shelf from '$lib/components/Shelf.svelte';

	let home = $state<Home | null>(null);
	let error = $state('');

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
	const greeting = $derived(session.user?.display_name ? `Hello, ${session.user.display_name}` : 'Hello');
</script>

<main class="page">
	<h1>{greeting}</h1>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#if home}
		<Shelf title="Continue listening" tiles={home.continue} wide />
		<Shelf title="Next in your series" tiles={home.series_next.map((s) => s.book)} />
		<Shelf title="Recently added" tiles={home.recently_added} />
		{#if !home.continue.length && !home.recently_added.length}
			<p class="muted">Nothing here yet. Browse the <a href="#/library">library</a> to start a book.</p>
		{/if}
	{:else if session.status === 'signed-in' && !error}
		<p class="muted">Loading…</p>
	{/if}
	<Shelf title="On this device" tiles={onDevice} />
</main>
