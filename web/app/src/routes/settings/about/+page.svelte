<script lang="ts">
	import { version } from '$app/environment';
	import { bytes } from '$lib/format';
	import { downloads } from '$lib/offline/downloads.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import { playbackLog, sendReport } from '$lib/player/log';

	$effect(() => void downloads.refreshStorage());

	const log = playbackLog();
	let sent = $state<'idle' | 'sending' | 'sent' | 'failed'>('idle');
	async function send() {
		sent = 'sending';
		sent = (await sendReport()) ? 'sent' : 'failed';
	}
</script>

<main class="page narrow">
	<button class="icon-btn back" onclick={() => history.back()} aria-label="Back"><Icon name="back" /></button>
	<span class="eyebrow accent">Settings</span>
	<h1>About ABShare</h1>
	<p class="lede serif">A private audiobook server for friends and family.</p>
	<p class="muted">App version {version}</p>
	<p>Made by James Muller (JameZUK): hacker and coder.</p>
	<h2>Why it exists</h2>
	<p>Because your audiobooks shouldn’t have to orbit Jeff Bezos to reach your ears. Amazon owns Audible; ABShare is owned by whoever runs it, and it answers to nobody’s shareholders.</p>
	<p>Bezos spent years and billions on rockets. New Shepard pops up past the edge of space for about ten minutes, then falls back down again, and New Glenn reached orbit years after it was first promised. ABShare reaches every device in your house and keeps your place in every book. It also launches on time, and it’s a far less embarrassing shape.</p>
	<p class="muted">No billionaires were consulted in the making of this app. None of your listening data goes to Amazon, and none of it goes into orbit.</p>
	<h2>Privacy</h2>
	<p>ABShare keeps what you listen to and where you are in each book, so you can carry on from any device. Nothing is shared with anyone else, and the app loads nothing from other websites.</p>
	<h2>Storage on this device</h2>
	{#if downloads.storage}
		<p>{bytes(downloads.storage.usage)} used of {bytes(downloads.storage.quota)} available to ABShare.</p>
		<p class="muted">{downloads.storage.persisted ? 'Your browser keeps downloads until you remove them.' : 'Your browser may clear downloads when space runs low. On iPhone and iPad, add ABShare to your Home Screen to keep them.'}</p>
	{:else}
		<p class="muted">This browser doesn’t report storage.</p>
	{/if}
	<h2>Playback report</h2>
	<p>If the book stops by itself in the background, or keeps playing through calls, send your admin a report of what the player saw: times, places in the book and what happened. It doesn’t say which book.</p>
	{#if log.length}
		<details>
			<summary>What will be sent ({log.length} lines)</summary>
			<pre class="log">{log.join('\n')}</pre>
		</details>
		<p>
			<button class="btn" onclick={send} disabled={sent === 'sending' || sent === 'sent'}>{sent === 'sent' ? 'Report sent' : 'Send a playback report'}</button>
		</p>
		{#if sent === 'sent'}<p class="muted" role="status">Sent. Your admin can read it in Admin → Overview, under Playback reports.</p>{/if}
		{#if sent === 'failed'}<p class="muted" role="status">That didn’t send. Check you’re online and try again in a minute.</p>{/if}
	{:else}
		<p class="muted">Nothing to report yet: play a book first.</p>
	{/if}
</main>

<style>
	.back {
		margin-left: -0.6rem;
	}
	.lede {
		font-size: 1.2rem;
		font-style: italic;
	}
	.log {
		font-size: 0.8rem;
		white-space: pre-wrap;
		overflow-wrap: anywhere;
		max-height: 20rem;
		overflow-y: auto;
	}
	h2 {
		margin-top: 2rem;
		padding-top: 1.25rem;
		border-top: 1px solid var(--line);
	}
</style>
