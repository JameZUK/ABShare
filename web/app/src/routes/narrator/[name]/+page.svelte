<script lang="ts">
	// Every book read by one narrator, with the library's search and sort.
	import { page } from '$app/state';
	import { personFromParam } from '$lib/people';
	import Icon from '$lib/components/Icon.svelte';
	import LibraryBrowser from '$lib/components/LibraryBrowser.svelte';

	const name = $derived(personFromParam(page.params.name));
</script>

{#if name}
	{#key name}
		<LibraryBrowser title="Read by {name}" narrator={name}>
			{#snippet header()}
				<p><button class="link" onclick={() => history.back()}><Icon name="back" size={16} /> Back</button></p>
			{/snippet}
		</LibraryBrowser>
	{/key}
{:else}
	<main class="page"><p class="error" role="alert">That link doesn’t work.</p></main>
{/if}

<style>
	.link {
		border: 0;
		background: none;
		color: inherit;
		padding: 0;
		display: inline-flex;
		gap: 0.25rem;
		align-items: center;
		min-height: 44px;
		cursor: pointer;
	}
</style>
