<script lang="ts">
	import { KBPS, SKIPS, SPEEDS, settings, type QualityMode } from '$lib/settings.svelte';
	import Icon from '$lib/components/Icon.svelte';

	const p = $derived(settings.prefs);
	const QUALITIES: [QualityMode, string, string][] = [
		['auto', 'Automatic', 'Data saver on mobile data where the browser can tell (Chrome on Android); otherwise Original.'],
		['original', 'Original', 'The book as it is in the library. Best quality.'],
		['saver', 'Data saver', 'A small mono stream, for mobile data. Uses about a third of the data.'],
	];
</script>

<main class="page narrow">
	<button class="icon-btn back" onclick={() => history.back()} aria-label="Back"><Icon name="back" /></button>
	<h1>Playback</h1>

	<fieldset>
		<legend class="eyebrow">Streaming quality</legend>
		<div class="rules group">
		{#each QUALITIES as [v, l, d] (v)}
			<label class="choice">
				<input type="radio" name="q" checked={p.quality === v} onchange={() => settings.set({ quality: v })} />
				<span><strong>{l}</strong><br /><span class="muted">{d}</span></span>
			</label>
		{/each}
		</div>
	</fieldset>

	<h2 class="eyebrow section">Data</h2>
	<div class="grid">
	<label class="field">Data saver bitrate
		<select value={p.saverKbps} onchange={(e) => settings.set({ saverKbps: Number(e.currentTarget.value) as 32 | 40 | 48 })}>
			{#each KBPS as k (k)}<option value={k}>{k} kb/s{k === 48 ? ' (clearest)' : k === 32 ? ' (smallest)' : ''}</option>{/each}
		</select>
	</label>

	<label class="field">Downloads
		<select value={p.downloadQuality} onchange={(e) => settings.set({ downloadQuality: e.currentTarget.value as 'original' | 'saver' })}>
			<option value="original">Original quality</option>
			<option value="saver">Data saver (smaller)</option>
		</select>
	</label>

	</div>
	<h2 class="eyebrow section">Listening</h2>
	<div class="grid">
	<label class="field">Skip back
		<select value={p.skipBack} onchange={(e) => settings.set({ skipBack: Number(e.currentTarget.value) })}>
			{#each SKIPS as s (s)}<option value={s}>{s} seconds</option>{/each}
		</select>
	</label>
	<label class="field">Skip forward
		<select value={p.skipForward} onchange={(e) => settings.set({ skipForward: Number(e.currentTarget.value) })}>
			{#each SKIPS as s (s)}<option value={s}>{s} seconds</option>{/each}
		</select>
	</label>
	<label class="field">Default speed
		<select value={p.speed} onchange={(e) => settings.set({ speed: Number(e.currentTarget.value) })}>
			{#each SPEEDS as s (s)}<option value={s}>{s}×</option>{/each}
		</select>
	</label>
	</div>
	<label class="choice toggle">
		<input type="checkbox" checked={p.autoRewind} onchange={(e) => settings.set({ autoRewind: e.currentTarget.checked })} />
		<span><strong>Rewind a little after a pause</strong><br /><span class="muted">A few seconds after a long pause, so you don’t lose the thread.</span></span>
	</label>
</main>

<style>
	.back {
		margin-left: -0.6rem;
	}
	fieldset {
		border: 0;
		padding: 0;
		margin: 0 0 1.5rem;
		min-width: 0;
	}
	legend {
		margin-bottom: 0.6rem;
	}
	.group {
		border-top: 1px solid var(--line);
		border-bottom: 1px solid var(--line);
	}
	.section {
		margin: 1.75rem 0 0.75rem;
		font-family: var(--font);
	}
	.grid {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
		gap: 0 1rem;
	}
	.choice {
		display: flex;
		gap: 0.9rem;
		align-items: flex-start;
		padding: 0.85rem 0;
		min-height: 44px;
		cursor: pointer;
	}
	.choice input {
		margin-top: 0.25rem;
		width: 1.2rem;
		height: 1.2rem;
		flex: none;
	}
	.choice .muted {
		font-size: 0.875rem;
	}
	.toggle {
		border-top: 1px solid var(--line);
		border-bottom: 1px solid var(--line);
		margin-top: 0.5rem;
	}
</style>
