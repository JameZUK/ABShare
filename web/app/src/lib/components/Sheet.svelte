<script lang="ts">
	// A modal sheet (bottom on phones, centred on wide screens) on the native <dialog>, which
	// traps focus and closes with Escape.
	import type { Snippet } from 'svelte';
	import Icon from './Icon.svelte';

	let { open = $bindable(false), title, children }: { open?: boolean; title: string; children: Snippet } = $props();
	let dialog: HTMLDialogElement | undefined = $state();

	$effect(() => {
		if (!dialog) return;
		if (open && !dialog.open) dialog.showModal();
		if (!open && dialog.open) dialog.close();
	});
</script>

<dialog
	bind:this={dialog}
	onclose={() => (open = false)}
	onclick={(e) => {
		if (e.target === dialog) open = false;
	}}
	aria-label={title}
>
	<div class="inner">
		<header class="row">
			<h2>{title}</h2>
			<span class="spacer"></span>
			<button class="icon-btn" onclick={() => (open = false)} aria-label="Close"><Icon name="close" /></button>
		</header>
		{#if open}{@render children()}{/if}
	</div>
</dialog>

<style>
	dialog {
		border: 0;
		padding: 0;
		background: var(--bg-raised);
		color: var(--fg);
		border-radius: var(--radius) var(--radius) 0 0;
		width: 100%;
		max-width: 520px;
		max-height: 85vh;
		margin: auto auto 0;
		box-shadow: var(--shadow);
	}
	@media (min-width: 700px) {
		dialog {
			margin: auto;
			border-radius: var(--radius);
		}
	}
	dialog::backdrop {
		background: rgb(0 0 0 / 45%);
	}
	.inner {
		padding: 0.75rem 1rem calc(1rem + var(--safe-b));
	}
	header h2 {
		margin: 0;
	}
</style>
