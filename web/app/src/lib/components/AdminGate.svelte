<script lang="ts">
	// Shows its content only to admins (the server checks every request anyway).
	import type { Snippet } from 'svelte';
	import { session } from '$lib/api/session.svelte';
	let { children }: { children: Snippet } = $props();
</script>

{#if session.user?.role === 'admin'}
	{@render children()}
{:else}
	<main class="page narrow"><p>Only admins can see this page.</p></main>
{/if}
