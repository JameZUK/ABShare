<script lang="ts">
	// Downloads: the quality new downloads use, holding them for Wi-Fi, the space they take,
	// and removing them all from this device.
	import { bytes } from '$lib/format';
	import { downloads } from '$lib/offline/downloads.svelte';
	import { connection, typeKnown } from '$lib/offline/wifi';
	import { settings } from '$lib/settings.svelte';
	import Icon from '$lib/components/Icon.svelte';

	const p = $derived(settings.prefs);
	const known = typeKnown(connection());
	let confirming = $state(false);
	let removing = $state(false);
	let done = $state('');

	$effect(() => void downloads.refreshStorage());

	function setWifiOnly(on: boolean) {
		settings.set({ wifiOnly: on });
		void downloads.recheck();
	}

	// Book by book, not downloads.removeAll: that wipes this person's whole on-device store
	// (for signing out), queued progress included.
	async function removeAll() {
		removing = true;
		try {
			for (const d of [...downloads.list]) await downloads.remove(d.id);
			await downloads.refreshStorage();
			done = 'All downloads were removed from this device.';
		} finally {
			removing = false;
			confirming = false;
		}
	}
</script>

<main class="page narrow">
	<button class="icon-btn back" onclick={() => history.back()} aria-label="Back"><Icon name="back" /></button>
	<h1>Downloads</h1>

	<fieldset>
		<legend class="eyebrow">Quality for new downloads</legend>
		<div class="rules group">
			<label class="choice">
				<input type="radio" name="dq" checked={p.downloadQuality === 'original'} onchange={() => settings.set({ downloadQuality: 'original' })} />
				<span><strong>Original</strong><br /><span class="muted">The book as it is in the library. Best quality, largest.</span></span>
			</label>
			<label class="choice">
				<input type="radio" name="dq" checked={p.downloadQuality === 'saver'} onchange={() => settings.set({ downloadQuality: 'saver' })} />
				<span><strong>Data saver</strong><br /><span class="muted">A small mono copy at {p.saverKbps} kb/s (about {Math.round((p.saverKbps * 3600) / 8 / 1000)} MB an hour). You can still choose for each book.</span></span>
			</label>
		</div>
	</fieldset>

	<fieldset>
		<legend class="eyebrow">Mobile data</legend>
		<label class="choice">
			<input type="checkbox" checked={p.wifiOnly} onchange={(e) => setWifiOnly(e.currentTarget.checked)} />
			<span>
				<strong>Download only on Wi-Fi</strong><br />
				{#if known}
					<span class="muted">On mobile data, downloads wait and carry on by themselves when you’re back on Wi-Fi.{p.wifiOnly && downloads.holding ? ' You’re on mobile data now, so they’re waiting.' : ''}</span>
				{:else}
					<span class="muted">This browser doesn’t say whether you’re on Wi-Fi, so downloads aren’t held.</span>
				{/if}
			</span>
		</label>
	</fieldset>

	<fieldset>
		<legend class="eyebrow">On this device</legend>
		{#if downloads.storage}
			<span class="meter" aria-hidden="true"><span style:width="{Math.min(100, (downloads.storage.usage / Math.max(1, downloads.storage.quota)) * 100)}%"></span></span>
			<p class="muted small">
				<strong>{bytes(downloads.storage.usage)}</strong> used of {bytes(downloads.storage.quota)} · {downloads.list.length} {downloads.list.length === 1 ? 'book' : 'books'}.
				{downloads.storage.persisted ? 'Kept until you remove them.' : 'Your browser may clear these when space runs low.'}
			</p>
		{/if}
		{#if done}<p role="status">{done}</p>{/if}
		{#if confirming}
			<p>Remove every download from this device? Your place in each book is kept.</p>
			<div class="row">
				<button class="btn danger" onclick={removeAll} disabled={removing}>Remove all</button>
				<button class="btn soft" onclick={() => (confirming = false)}>Keep them</button>
			</div>
		{:else}
			<button class="btn soft" onclick={() => { done = ''; confirming = true; }} disabled={!downloads.list.length}>Remove all downloads…</button>
		{/if}
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
		margin-bottom: 0.6rem;
	}
	.group {
		border-top: 1px solid var(--line);
		border-bottom: 1px solid var(--line);
	}
	.choice {
		display: flex;
		gap: 0.85rem;
		align-items: flex-start;
		padding: 0.8rem 0;
		min-height: 44px;
		cursor: pointer;
	}
	.choice input {
		margin-top: 0.25rem;
		width: 1.2rem;
		height: 1.2rem;
		flex: none;
	}
	.meter {
		display: block;
		height: 4px;
		border-radius: 2px;
		background: var(--line);
		overflow: hidden;
		margin-bottom: 0.5rem;
	}
	.meter span {
		display: block;
		height: 100%;
		background: var(--fg);
	}
	.small {
		font-size: 0.875rem;
	}
	.small strong {
		color: var(--fg);
	}
</style>
