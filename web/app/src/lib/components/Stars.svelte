<script lang="ts">
	// Star ratings, 1 to 5. Read-only it shows a value (an average is shown to the nearest
	// half star); with `input` it is a group of five real radio buttons, so it works with the
	// keyboard and screen readers like any other choice.
	let {
		value = $bindable(0),
		input = false,
		size = 18,
		name = 'rating',
		label = 'Rating',
	}: { value?: number; input?: boolean; size?: number; name?: string; label?: string } = $props();

	const STAR = 'M12 3.2l2.6 5.6 6.1.7-4.5 4.2 1.2 6-5.4-3.1-5.4 3.1 1.2-6L3.3 9.5l6.1-.7z';
	const shown = $derived(Math.min(5, Math.max(0, Math.round(value * 2) / 2)));
	const spoken = $derived(`${shown % 1 ? shown.toFixed(1) : shown} out of 5 stars`);
	let hover = $state(0);
	const lit = (n: number) => (hover || value) >= n;
</script>

{#if input}
	<div class="input" role="radiogroup" aria-label={label} style:--s="{size}px">
		{#each [1, 2, 3, 4, 5] as n (n)}
			<label class:lit={lit(n)} onpointerenter={() => (hover = n)} onpointerleave={() => (hover = 0)}>
				<input type="radio" {name} value={n} checked={value === n} onchange={() => (value = n)} aria-label={n === 1 ? '1 star' : `${n} stars`} />
				<svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true"><path d={STAR} /></svg>
			</label>
		{/each}
	</div>
{:else}
	<span class="show" role="img" aria-label={spoken} style:--s="{size}px">
		<span class="row empty" aria-hidden="true">
			{#each [1, 2, 3, 4, 5] as n (n)}<svg width={size} height={size} viewBox="0 0 24 24"><path d={STAR} /></svg>{/each}
		</span>
		<span class="row full" aria-hidden="true" style:width="{(shown / 5) * 100}%">
			{#each [1, 2, 3, 4, 5] as n (n)}<svg width={size} height={size} viewBox="0 0 24 24"><path d={STAR} /></svg>{/each}
		</span>
	</span>
{/if}

<style>
	.show {
		position: relative;
		display: inline-block;
		/* Its own width, never stretched by a grid or flex parent: the filled row's width is a
		   share of it. */
		width: max-content;
		justify-self: start;
		align-self: start;
		flex: none;
		vertical-align: -0.15em;
		line-height: 0;
	}
	.row {
		display: inline-flex;
		gap: 1px;
	}
	.full {
		position: absolute;
		left: 0;
		top: 0;
		overflow: hidden;
		white-space: nowrap;
	}
	.full svg {
		flex: none;
	}
	.empty path {
		fill: none;
		stroke: var(--fg-muted);
		stroke-width: 1.4;
		stroke-linejoin: round;
	}
	.full path {
		fill: var(--accent);
		stroke: var(--accent);
		stroke-width: 1.4;
		stroke-linejoin: round;
	}
	.input {
		display: inline-flex;
	}
	.input label {
		position: relative;
		display: grid;
		place-items: center;
		width: 44px;
		height: 44px;
		cursor: pointer;
	}
	.input input {
		position: absolute;
		inset: 0;
		margin: 0;
		opacity: 0;
		cursor: pointer;
	}
	.input path {
		fill: none;
		stroke: var(--fg-muted);
		stroke-width: 1.4;
		stroke-linejoin: round;
		transition: fill 0.12s var(--ease);
	}
	.input label.lit path {
		fill: var(--accent);
		stroke: var(--accent);
	}
	.input label:has(input:focus-visible) {
		outline: 2px solid var(--accent);
		outline-offset: -4px;
		border-radius: 50%;
	}
</style>
