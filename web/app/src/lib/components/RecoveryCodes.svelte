<script lang="ts">
	let { codes }: { codes: string[] } = $props();
	let copied = $state(false);

	async function copy() {
		try {
			await navigator.clipboard.writeText(codes.join('\n'));
			copied = true;
		} catch {
			copied = false;
		}
	}
</script>

<ol class="codes">
	{#each codes as c (c)}<li><code>{c}</code></li>{/each}
</ol>
<button class="btn soft" onclick={copy}>{copied ? 'Copied' : 'Copy codes'}</button>

<style>
	.codes {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(10rem, 1fr));
		gap: 0.5rem 1.25rem;
		margin: 1rem 0;
		padding: 1.1rem 1.1rem 1.1rem 2.4rem;
		background: var(--bg-raised);
		border: 1px dashed var(--line);
		border-radius: var(--radius-sm);
		font-size: 1.05rem;
		user-select: all;
	}
	.codes li::marker {
		color: var(--fg-muted);
		font-size: 0.85rem;
	}
	code {
		font-variant-numeric: tabular-nums;
		letter-spacing: 0.02em;
	}
	button {
		margin-bottom: 1rem;
	}
</style>
