<script lang="ts">
	// Shows its content only to admins (the server checks every request anyway).
	import type { Snippet } from 'svelte';
	import { session } from '$lib/api/session.svelte';
	let { children }: { children: Snippet } = $props();
</script>

{#if session.user?.role === 'admin'}
	{@render children()}
{:else}
	<main class="page narrow"><p class="muted gate">Only admins can see this page.</p></main>
{/if}

<style>
	.gate {
		margin-top: 3rem;
		text-align: center;
		font-family: var(--font-display);
		font-style: italic;
		font-size: 1.15rem;
	}
</style>
