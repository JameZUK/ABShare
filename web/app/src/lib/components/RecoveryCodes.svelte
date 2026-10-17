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
		gap: 0.4rem 1rem;
		padding: 1rem 1rem 1rem 2.2rem;
		background: var(--bg-sunken);
		border-radius: var(--radius-sm);
		font-size: 1.05rem;
		user-select: all;
	}
	button {
		margin-bottom: 1rem;
	}
</style>
