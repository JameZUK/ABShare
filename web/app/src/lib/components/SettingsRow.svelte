<script lang="ts">
	import Icon from './Icon.svelte';
	let { href, icon, title, summary }: { href: string; icon: string; title: string; summary: string } = $props();
</script>

<a class="row-link" {href}>
	<Icon name={icon} />
	<span class="text"><span class="t">{title}</span><span class="muted s">{summary}</span></span>
	<Icon name="right" size={18} />
</a>

<style>
	.row-link {
		display: flex;
		align-items: center;
		gap: 1rem;
		min-height: 60px;
		padding: 0.5rem 0.25rem;
		color: inherit;
		text-decoration: none;
		border-bottom: 1px solid var(--line);
	}
	.text {
		flex: 1;
		display: grid;
		min-width: 0;
	}
	.t {
		font-weight: 600;
	}
	.s {
		font-size: 0.85rem;
	}
</style>
