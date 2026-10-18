<script lang="ts">
	import { appearance } from '$lib/theme/appearance.svelte';
	import { THEMES, type Mode } from '$lib/theme/themes';
	import Icon from '$lib/components/Icon.svelte';

	const MODES: [Mode, string][] = [
		['auto', 'System'],
		['light', 'Light'],
		['dark', 'Dark'],
	];
</script>

<main class="page narrow">
	<button class="icon-btn back" onclick={() => history.back()} aria-label="Back"><Icon name="back" /></button>
	<h1>Appearance</h1>

	<fieldset>
		<legend class="eyebrow">Light or dark</legend>
		<div class="segmented">
			{#each MODES as [m, l] (m)}
				<label><input type="radio" name="mode" checked={appearance.prefs.mode === m} onchange={() => appearance.set({ mode: m })} />{l}</label>
			{/each}
		</div>
		<p class="muted hint">System follows your device, switching to dark at night if it does.</p>
	</fieldset>

	<fieldset>
		<legend class="eyebrow">Theme</legend>
		<div class="rules themes">
			{#each THEMES as t (t.id)}
				<label class="choice">
					<input type="radio" name="theme" checked={appearance.prefs.theme === t.id} onchange={() => appearance.set({ theme: t.id })} />
					<span class="sample {t.id}" aria-hidden="true"><span class="aa">Aa</span></span>
					<span class="text"><strong>{t.name}</strong><span class="muted">{t.description}</span></span>
				</label>
			{/each}
		</div>
	</fieldset>
</main>

<style>
	.back {
		margin-left: -0.6rem;
	}
	fieldset {
		border: 0;
		padding: 0;
		margin: 0 0 2rem;
		min-width: 0;
	}
	legend {
		margin-bottom: 0.75rem;
	}
	.hint {
		font-size: 0.875rem;
		margin-top: 0.6rem;
	}
	.themes {
		border-top: 1px solid var(--line);
		border-bottom: 1px solid var(--line);
	}
	.choice {
		display: flex;
		gap: 1rem;
		align-items: center;
		padding: 0.9rem 0;
		min-height: 44px;
		cursor: pointer;
	}
	.choice input {
		width: 1.2rem;
		height: 1.2rem;
		flex: none;
	}
	.sample {
		width: 52px;
		height: 52px;
		flex: none;
		display: grid;
		place-items: center;
		border-radius: var(--radius-sm);
		border: 1px solid var(--line);
	}
	.sample.paperback {
		background: #f3eee4;
		color: #1e2430;
		font-family: var(--font-display);
		box-shadow: inset -26px 0 0 #17140f;
	}
	.sample.paperback .aa {
		background: linear-gradient(90deg, #1e2430 50%, #ece4d4 50%);
		background-clip: text;
		color: transparent;
	}
	.sample.contrast {
		background: #000000;
		color: #ffd60a;
		font-weight: 700;
		border: 2px solid var(--fg);
	}
	.aa {
		font-size: 1.35rem;
		font-weight: 600;
	}
	.text {
		display: grid;
		gap: 0.1rem;
	}
	.text .muted {
		font-size: 0.875rem;
	}
</style>
