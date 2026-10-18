<script lang="ts">
	// A book cover: from the server, or from the device when the book is downloaded and the
	// server can't be reached. Falls back to a plain jacket with the title. `book` draws it as
	// a physical book: a shaded spine on the left and a soft shadow underneath.
	import { session } from '$lib/api/session.svelte';
	import { downloads } from '$lib/offline/downloads.svelte';

	let {
		id,
		src,
		title,
		size = 320,
		book = false,
	}: { id: string; src: string | null; title: string; size?: 128 | 320 | 640 | 1280; book?: boolean } = $props();
	let failed = $state(false);
	let blobUrl = $state<string | null>(null);

	const online = $derived(session.status === 'signed-in');
	const url = $derived(online && src && !failed ? `v1/${src}?size=${size}` : blobUrl);

	$effect(() => {
		// Offline (or the server's copy failed): use the stored cover if there is one.
		if ((online && !failed) || !downloads.playable(id)) return;
		let made: string | null = null;
		let live = true;
		void downloads.cover(id).then((b) => {
			if (b && live) blobUrl = made = URL.createObjectURL(b);
		});
		return () => {
			live = false;
			if (made) URL.revokeObjectURL(made);
		};
	});

	// A stable, quiet colour per book for the fallback jacket.
	const hue = $derived([...id].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7));
</script>

<span class="frame" class:book>
	{#if url}
		<img class="cover" src={url} alt="" loading="lazy" decoding="async" onerror={() => (failed = true)} />
	{:else}
		<span class="cover fallback" style:--h={hue} aria-hidden="true"><span>{title}</span></span>
	{/if}
</span>

<style>
	.frame {
		position: relative;
		display: block;
		width: 100%;
		aspect-ratio: 1;
		border-radius: var(--radius-sm);
		overflow: hidden;
		background: var(--bg-sunken);
	}
	.frame.book {
		border-radius: 3px var(--radius-sm) var(--radius-sm) 3px;
		box-shadow: var(--shadow-book);
	}
	/* The spine: a darker strip with a faint highlight where the cover would fold. */
	.frame.book::after {
		content: '';
		position: absolute;
		inset: 0;
		pointer-events: none;
		background: linear-gradient(
			90deg,
			rgb(0 0 0 / 26%) 0,
			rgb(0 0 0 / 10%) 2.5%,
			rgb(255 255 255 / 12%) 3.5%,
			rgb(255 255 255 / 0%) 6%
		);
	}
	.cover {
		display: block;
		width: 100%;
		height: 100%;
		object-fit: cover;
	}
	.fallback {
		display: flex;
		align-items: flex-end;
		padding: 12% 12% 14% 16%;
		background: hsl(var(--h) 28% 32%);
		color: #f6f1e7;
		font-family: var(--font-display);
		font-weight: 600;
		line-height: 1.1;
		overflow: hidden;
	}
	.fallback span {
		display: -webkit-box;
		-webkit-line-clamp: 4;
		line-clamp: 4;
		-webkit-box-orient: vertical;
		overflow: hidden;
		font-size: clamp(0.75rem, 2.6vw, 1.1rem);
	}
</style>
