<script lang="ts">
	import '../app.css';
	import { page } from '$app/state';
	import { api, session } from '$lib/api/session.svelte';
	import type { BookDetail } from '$lib/api/types';
	import { appearance } from '$lib/theme/appearance.svelte';
	import { updates } from '$lib/pwa/update.svelte';
	import { downloads } from '$lib/offline/downloads.svelte';
	import { sync } from '$lib/offline/sync.svelte';
	import { lastBook, player } from '$lib/player/player.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import MiniPlayer from '$lib/components/MiniPlayer.svelte';
	import PlayerView from '$lib/components/PlayerView.svelte';
	import SignIn from '$lib/components/SignIn.svelte';

	let { children } = $props();

	appearance.start();
	updates.start();
	player.start();
	session.onEnd = (reason, user) => {
		player.close();
		if (reason === 'revoked' && user) void downloads.removeAll(user);
	};
	void session.start();

	const inviting = $derived(page.url.hash.startsWith('#/invite/'));
	const ready = $derived(session.status === 'signed-in' || session.status === 'offline');

	// After sign-in (or with the last user, offline): load downloads, flush the outbox and
	// bring back the last book, paused.
	let restoredFor: string | null = null;
	$effect(() => {
		const user = session.user?.username;
		if (!ready || !user || restoredFor === user) return;
		restoredFor = user;
		void downloads.load();
		void sync.flush();
		const id = lastBook();
		if (id && !player.book) void restore(id);
	});

	$effect(() => {
		if (session.status === 'signed-in') {
			void sync.flush();
			void downloads.load();
		}
	});

	async function restore(id: string) {
		let detail: BookDetail | undefined;
		try {
			detail = session.status === 'signed-in' ? await api<BookDetail>(`v1/books/${id}`) : undefined;
		} catch {
			detail = undefined;
		}
		detail ??= await downloads.detail(id);
		if (detail) await player.open(detail, { play: false });
	}

	const TABS = [
		{ href: '#/', label: 'Home', icon: 'home', match: (h: string) => h === '' || h === '#/' },
		{ href: '#/library', label: 'Library', icon: 'library', match: (h: string) => h.startsWith('#/library') || h.startsWith('#/group') || h.startsWith('#/genre') },
		{ href: '#/downloads', label: 'Downloads', icon: 'downloaded', match: (h: string) => h.startsWith('#/downloads') },
		{ href: '#/settings', label: 'Settings', icon: 'settings', match: (h: string) => h.startsWith('#/settings') || h.startsWith('#/admin') },
	];
</script>

{#if inviting}
	{@render children()}
{:else if session.status === 'starting'}
	<div class="splash" aria-busy="true"><img src="./favicon.svg" alt="ABShare" width="72" height="72" /></div>
{:else if !ready}
	<SignIn />
{:else}
	{#if session.status === 'offline'}
		<p class="banner" role="status"><Icon name="offline" size={18} /> Offline. Downloaded books still play.</p>
	{:else if updates.available}
		<p class="banner" role="status">An update is ready. <button class="link" onclick={() => updates.apply()}>Reload</button></p>
	{/if}
	{@render children()}
	<MiniPlayer />
	<nav class="tabs" aria-label="Main">
		{#each TABS as t (t.href)}
			<a href={t.href} aria-current={t.match(page.url.hash) ? 'page' : undefined}>
				<Icon name={t.icon} />
				<span>{t.label}</span>
			</a>
		{/each}
	</nav>
	{#if player.expanded}<PlayerView />{/if}
{/if}

<style>
	.splash {
		display: grid;
		place-items: center;
		height: var(--app-h);
	}
	.banner {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 0.5em;
		margin: 0;
		padding: 0.5rem 1rem;
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
		font-size: 0.9rem;
	}
	.link {
		border: 0;
		background: none;
		color: inherit;
		font-weight: 700;
		text-decoration: underline;
		min-height: 44px;
	}
	.tabs {
		position: fixed;
		left: 0;
		right: 0;
		bottom: 0;
		height: calc(var(--tabs-h) + var(--safe-b));
		padding-bottom: var(--safe-b);
		display: grid;
		grid-template-columns: repeat(4, 1fr);
		background: var(--bg-raised);
		border-top: 1px solid var(--line);
		z-index: 20;
	}
	.tabs a {
		display: grid;
		justify-items: center;
		align-content: center;
		gap: 2px;
		color: var(--fg-muted);
		text-decoration: none;
		font-size: 0.75rem;
	}
	.tabs a[aria-current='page'] {
		color: var(--accent);
		font-weight: 600;
	}
</style>
