<script lang="ts">
	import { appearance } from '$lib/theme/appearance.svelte';
	import { ACCENTS, THEMES, type Mode } from '$lib/theme/themes';
	import Icon from '$lib/components/Icon.svelte';

	const MODES: [Mode, string][] = [
		['auto', 'Follow the system'],
		['light', 'Light'],
		['dark', 'Dark'],
	];
	const accentOk = $derived(THEMES.find((t) => t.id === appearance.prefs.theme)?.accent ?? false);
</script>

<main class="page narrow">
	<button class="icon-btn" onclick={() => history.back()} aria-label="Back"><Icon name="back" /></button>
	<h1>Appearance</h1>
	<fieldset>
		<legend>Theme</legend>
		{#each THEMES as t (t.id)}
			<label class="choice">
				<input type="radio" name="theme" checked={appearance.prefs.theme === t.id} onchange={() => appearance.set({ theme: t.id })} />
				<span><strong>{t.name}</strong><br /><span class="muted">{t.description}</span></span>
			</label>
		{/each}
	</fieldset>
	<fieldset>
		<legend>Light or dark</legend>
		{#each MODES as [m, l] (m)}
			<label class="choice"><input type="radio" name="mode" checked={appearance.prefs.mode === m} onchange={() => appearance.set({ mode: m })} /> {l}</label>
		{/each}
	</fieldset>
	{#if accentOk}
		<fieldset>
			<legend>Accent colour</legend>
			<div class="swatches">
				{#each ACCENTS as a (a.hex)}
					<button class="swatch" style:background={a.hex} aria-label={a.name} aria-pressed={appearance.prefs.accent === a.hex} onclick={() => appearance.set({ accent: a.hex })}></button>
				{/each}
			</div>
		</fieldset>
	{/if}
</main>

<style>
	fieldset {
		border: 0;
		padding: 0;
		margin: 0 0 1.25rem;
	}
	legend {
		font-weight: 600;
		margin-bottom: 0.5rem;
	}
	.choice {
		display: flex;
		gap: 0.75rem;
		align-items: flex-start;
		padding: 0.4rem 0;
		min-height: 44px;
	}
	.choice input {
		margin-top: 0.3rem;
		width: 1.2rem;
		height: 1.2rem;
		accent-color: var(--accent);
	}
	.swatches {
		display: flex;
		flex-wrap: wrap;
		gap: 0.6rem;
	}
	.swatch {
		width: 44px;
		height: 44px;
		border-radius: 50%;
		border: 3px solid transparent;
	}
	.swatch[aria-pressed='true'] {
		border-color: var(--fg);
	}
</style>
