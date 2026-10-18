<script lang="ts">
	import { page } from '$app/state';
	import Icon from './Icon.svelte';
	let { title }: { title: string } = $props();
	const LINKS = [
		['#/admin', 'Overview'],
		['#/admin/users', 'Users'],
		['#/admin/invites', 'Invites'],
		['#/admin/library', 'Library'],
		['#/admin/jobs', 'Jobs'],
		['#/admin/audit', 'Audit log'],
	] as const;
</script>

<a class="icon-btn back" href="#/settings" aria-label="Back to settings"><Icon name="back" /></a>
<span class="eyebrow">Admin</span>
<h1>{title}</h1>
<nav class="chips tabs" aria-label="Admin">
	{#each LINKS as [href, label] (href)}
		<a class="chip" {href} aria-current={page.url.hash === href ? 'page' : undefined}>{label}</a>
	{/each}
</nav>

<style>
	.back {
		margin-left: -0.6rem;
	}
	.tabs {
		flex-wrap: nowrap;
		overflow-x: auto;
		scrollbar-width: none;
		margin: 0 calc(-1 * var(--gutter)) 1.5rem;
		padding: 0 var(--gutter) 1.25rem;
		border-bottom: 1px solid var(--line);
	}
	.tabs::-webkit-scrollbar {
		display: none;
	}
	.chip {
		flex: none;
		min-height: 40px;
	}
	@media (min-width: 1024px) {
		.tabs {
			margin-left: 0;
			margin-right: 0;
			padding-left: 0;
			padding-right: 0;
		}
	}
</style>
