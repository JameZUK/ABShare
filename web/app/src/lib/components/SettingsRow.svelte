<script lang="ts">
	import Icon from './Icon.svelte';
	let { href, icon, title, summary }: { href: string; icon: string; title: string; summary: string } = $props();
</script>

<a class="row-link" {href}>
	<span class="ic"><Icon name={icon} size={20} /></span>
	<span class="text"><span class="t serif">{title}</span><span class="muted s">{summary}</span></span>
	<span class="chev"><Icon name="right" size={18} /></span>
</a>

<style>
	.row-link {
		display: flex;
		align-items: center;
		gap: 1rem;
		min-height: 68px;
		padding: 0.75rem 0;
		color: inherit;
		text-decoration: none;
	}
	.ic {
		display: grid;
		place-items: center;
		width: 40px;
		height: 40px;
		flex: none;
		border: 1px solid var(--line);
		border-radius: 50%;
	}
	.text {
		flex: 1;
		display: grid;
		gap: 0.1rem;
		min-width: 0;
	}
	.t {
		font-size: 1.1rem;
		font-weight: 600;
	}
	.s {
		font-size: 0.85rem;
	}
	.chev {
		color: var(--fg-muted);
		display: grid;
		transition: transform 0.15s var(--ease);
	}
	.row-link:hover .chev {
		transform: translateX(2px);
		color: var(--fg);
	}
</style>
