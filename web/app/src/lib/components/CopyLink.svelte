<script lang="ts">
	// An invite or reset link, shown once, with a copy button.
	let { link, note }: { link: string; note: string } = $props();
	let copied = $state(false);
	async function copy() {
		try {
			await navigator.clipboard.writeText(link);
			copied = true;
		} catch {
			copied = false;
		}
	}
</script>

<div class="box" role="status">
	<p class="note">{note}</p>
	<code>{link}</code>
	<button class="btn soft" onclick={copy}>{copied ? 'Copied' : 'Copy link'}</button>
</div>

<style>
	.box {
		display: grid;
		gap: 0.75rem;
		padding: 1.1rem 1.2rem;
		border-radius: var(--radius-sm);
		background: var(--bg-raised);
		border: 1px solid var(--fg);
		margin-bottom: 1.25rem;
	}
	code {
		word-break: break-all;
		user-select: all;
		padding: 0.6rem 0.75rem;
		border-radius: 6px;
		background: var(--bg-sunken);
		font-size: 0.85rem;
	}
	.box p {
		margin: 0;
	}
	.note {
		font-weight: 600;
	}
	.btn {
		justify-self: start;
	}
</style>
