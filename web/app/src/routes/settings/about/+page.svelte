<script lang="ts">
	import { version } from '$app/environment';
	import { bytes } from '$lib/format';
	import { downloads } from '$lib/offline/downloads.svelte';
	import Icon from '$lib/components/Icon.svelte';

	$effect(() => void downloads.refreshStorage());
</script>

<main class="page narrow">
	<button class="icon-btn" onclick={() => history.back()} aria-label="Back"><Icon name="back" /></button>
	<h1>About ABShare</h1>
	<p>A private audiobook server for friends and family.</p>
	<p class="muted">App version {version}</p>
	<h2>Privacy</h2>
	<p>ABShare keeps what you listen to and where you are in each book, so you can carry on from any device. Nothing is shared with anyone else, and the app loads nothing from other websites.</p>
	<h2>Storage on this device</h2>
	{#if downloads.storage}
		<p>{bytes(downloads.storage.usage)} used of {bytes(downloads.storage.quota)} available to ABShare.</p>
		<p class="muted">{downloads.storage.persisted ? 'Your browser keeps downloads until you remove them.' : 'Your browser may clear downloads when space runs low. On iPhone and iPad, add ABShare to your Home Screen to keep them.'}</p>
	{:else}
		<p class="muted">This browser doesn’t report storage.</p>
	{/if}
</main>
