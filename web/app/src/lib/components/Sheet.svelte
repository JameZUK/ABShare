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
		<span class="handle" aria-hidden="true"></span>
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
		border-radius: 20px 20px 0 0;
		width: 100%;
		max-width: 520px;
		max-height: 85vh;
		margin: auto auto 0;
		box-shadow: var(--shadow);
	}
	dialog[open] {
		animation: sheet-up 0.28s var(--ease);
	}
	@keyframes sheet-up {
		from {
			transform: translateY(24px);
			opacity: 0;
		}
	}
	@media (min-width: 700px) {
		dialog {
			margin: auto;
			border-radius: var(--radius);
			border: 1px solid var(--line);
		}
	}
	dialog::backdrop {
		background: var(--scrim);
	}
	.inner {
		padding: 0.5rem 1.25rem calc(1.25rem + var(--safe-b));
	}
	.handle {
		display: block;
		width: 40px;
		height: 4px;
		margin: 0.25rem auto 0.5rem;
		border-radius: 2px;
		background: var(--line);
	}
	@media (min-width: 700px) {
		.handle {
			display: none;
		}
		.inner {
			padding-top: 1rem;
		}
	}
	header {
		margin-bottom: 0.75rem;
		padding-bottom: 0.6rem;
		border-bottom: 1px solid var(--line);
	}
	header h2 {
		margin: 0;
		font-size: 1.4rem;
	}
	header .icon-btn {
		margin-right: -0.6rem;
	}
</style>
