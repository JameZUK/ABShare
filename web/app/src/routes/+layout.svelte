<script lang="ts">
	import '../app.css';
	import { page } from '$app/state';
	import { api, session } from '$lib/api/session.svelte';
	import type { BookDetail } from '$lib/api/types';
	import { appearance } from '$lib/theme/appearance.svelte';
	import { cookiesBlocked, updates } from '$lib/pwa/update.svelte';
	import { downloads } from '$lib/offline/downloads.svelte';
	import { sync } from '$lib/offline/sync.svelte';
	import { settings } from '$lib/settings.svelte';
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
	// After a recovery-code sign-in only the account page works until a new factor is set up.
	const limited = $derived(session.status === 'signed-in' && session.user?.needs_factor === true);
	$effect(() => {
		if (limited && !page.url.hash.startsWith('#/settings/account')) location.hash = '#/settings/account';
	});

	// After sign-in (or with the last user, offline): load downloads, flush the outbox and
	// bring back the last book, paused.
	let restoredFor: string | null = null;
	$effect(() => {
		const user = session.user?.username;
		if (!ready || limited || !user || restoredFor === user) return;
		restoredFor = user;
		settings.owner = user;
		void downloads.finishWipes().then(() => downloads.load());
		void sync.flush();
		const id = lastBook();
		if (id && !player.book) void restore(id);
	});

	$effect(() => {
		if (session.status === 'signed-in' && !limited) {
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

{#if cookiesBlocked()}
	<p class="banner" role="alert">ABShare needs cookies to keep you signed in and to play books. Allow cookies and site data for this site in your browser’s settings, then reload.</p>
{/if}
{#if inviting}
	{@render children()}
{:else if session.status === 'starting'}
	<div class="splash" aria-busy="true"><img src="./favicon.svg" alt="ABShare" width="72" height="72" /></div>
{:else if !ready}
	<SignIn />
{:else if limited}
	{@render children()}
{:else}
	{#if session.status === 'offline'}
		<p class="banner" role="status"><Icon name="offline" size={18} /> Offline. Downloaded books still play.</p>
	{:else if updates.available}
		<p class="banner" role="status">An update is ready. <button class="link" onclick={() => updates.apply()}>Reload</button></p>
	{/if}
	<div class="shell">{@render children()}</div>
	<MiniPlayer />
	<nav class="tabs" aria-label="Main">
		<a class="brand" href="#/" aria-label="ABShare home"><img src="./favicon.svg" alt="" width="34" height="34" /><span class="serif">ABShare</span></a>
		{#each TABS as t (t.href)}
			<a class="tab" href={t.href} aria-current={t.match(page.url.hash) ? 'page' : undefined}>
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
		padding: 0.5rem 1rem 0.5rem calc(var(--side-w) + 1rem);
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
		background: var(--bg);
		border-top: 1px solid var(--line);
		z-index: 20;
	}
	.brand {
		display: none;
	}
	.tab {
		display: grid;
		justify-items: center;
		align-content: center;
		gap: 3px;
		color: var(--fg-muted);
		text-decoration: none;
		font-size: 0.72rem;
		font-weight: 500;
	}
	.tab[aria-current='page'] {
		color: var(--fg);
		font-weight: 600;
	}
	/* Wide screens: the tabs become a sidebar. */
	@media (min-width: 1024px) {
		.shell {
			padding-left: var(--side-w);
		}
		.tabs {
			top: 0;
			right: auto;
			width: var(--side-w);
			height: auto;
			padding: 1.5rem 1rem;
			display: flex;
			flex-direction: column;
			gap: 0.25rem;
			border-top: 0;
			border-right: 1px solid var(--line);
		}
		.brand {
			display: flex;
			align-items: center;
			gap: 0.7rem;
			padding: 0 0.75rem 1.75rem;
			color: var(--fg);
			text-decoration: none;
			font-size: 1.35rem;
			font-weight: 600;
		}
		.brand img {
			border-radius: 9px;
		}
		.tab {
			display: flex;
			align-items: center;
			gap: 0.85rem;
			min-height: 46px;
			padding: 0 0.85rem;
			border-radius: var(--radius-sm);
			font-size: 0.95rem;
		}
		.tab:hover {
			background: var(--bg-sunken);
			color: var(--fg);
		}
		.tab[aria-current='page'] {
			background: var(--bg-sunken);
		}
	}
</style>
