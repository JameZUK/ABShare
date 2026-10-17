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

<a class="icon-btn" href="#/settings" aria-label="Back to settings"><Icon name="back" /></a>
<h1>{title}</h1>
<nav class="tabs" aria-label="Admin">
	{#each LINKS as [href, label] (href)}
		<a {href} aria-current={page.url.hash === href ? 'page' : undefined}>{label}</a>
	{/each}
</nav>

<style>
	.tabs {
		display: flex;
		gap: 0.25rem;
		overflow-x: auto;
		margin-bottom: 1rem;
		border-bottom: 1px solid var(--line);
	}
	.tabs a {
		flex: none;
		padding: 0.6rem 0.9rem;
		min-height: 44px;
		color: var(--fg-muted);
		text-decoration: none;
		border-bottom: 3px solid transparent;
	}
	.tabs a[aria-current='page'] {
		color: var(--accent);
		border-color: var(--accent);
		font-weight: 600;
	}
</style>
