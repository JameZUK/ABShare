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
	<p>{note}</p>
	<code>{link}</code>
	<button class="btn soft" onclick={copy}>{copied ? 'Copied' : 'Copy link'}</button>
</div>

<style>
	.box {
		display: grid;
		gap: 0.6rem;
		padding: 1rem;
		border-radius: var(--radius-sm);
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
		margin-bottom: 1rem;
	}
	code {
		word-break: break-all;
		user-select: all;
	}
	.box p {
		margin: 0;
	}
	.btn {
		justify-self: start;
	}
</style>
