<script lang="ts">
	// A book cover: from the server, or from the device when the book is downloaded and the
	// server can't be reached. Falls back to a tile with the title.
	import { session } from '$lib/api/session.svelte';
	import { downloads } from '$lib/offline/downloads.svelte';

	let { id, src, title, size = 320 }: { id: string; src: string | null; title: string; size?: 128 | 320 | 640 | 1280 } = $props();
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

	// A stable, quiet colour per book for the fallback tile.
	const hue = $derived([...id].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7));
</script>

{#if url}
	<img class="cover" src={url} alt="" loading="lazy" decoding="async" onerror={() => (failed = true)} />
{:else}
	<div class="cover fallback" style:--h={hue} aria-hidden="true"><span>{title}</span></div>
{/if}

<style>
	.cover {
		width: 100%;
		aspect-ratio: 1;
		object-fit: cover;
		border-radius: var(--radius-sm);
		background: var(--bg-sunken);
	}
	.fallback {
		display: grid;
		place-items: center;
		padding: 12%;
		background: linear-gradient(135deg, hsl(var(--h) 35% 38%), hsl(calc(var(--h) + 40) 35% 24%));
		color: #fff;
		font-weight: 700;
		text-align: center;
		overflow: hidden;
	}
	.fallback span {
		display: -webkit-box;
		-webkit-line-clamp: 4;
		line-clamp: 4;
		-webkit-box-orient: vertical;
		overflow: hidden;
		font-size: clamp(0.7rem, 2.5vw, 1rem);
	}
</style>
