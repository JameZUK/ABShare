<script lang="ts">
	// Every book by one author, with the library's search and sort.
	import { page } from '$app/state';
	import { personFromParam } from '$lib/people';
	import LibraryBrowser from '$lib/components/LibraryBrowser.svelte';

	const name = $derived(personFromParam(page.params.name));
</script>

{#if name}
	{#key name}
		<LibraryBrowser title={name} author={name} eyebrow="Books by" back />
	{/key}
{:else}
	<main class="page"><p class="error" role="alert">That link doesn’t work.</p></main>
{/if}
