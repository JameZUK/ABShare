<script lang="ts">
	import LibraryBrowser from '$lib/components/LibraryBrowser.svelte';
	import GenreChips from '$lib/components/GenreChips.svelte';
</script>

<LibraryBrowser title="Library">
	{#snippet header()}<GenreChips />{/snippet}
</LibraryBrowser>
