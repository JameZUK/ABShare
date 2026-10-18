<script lang="ts">
	// Settings: an index of rows, each with a one-line summary (SPEC §7.2).
	import { session } from '$lib/api/session.svelte';
	import { settings } from '$lib/settings.svelte';
	import { appearance } from '$lib/theme/appearance.svelte';
	import { THEMES } from '$lib/theme/themes';
	import SettingsRow from '$lib/components/SettingsRow.svelte';

	const p = $derived(settings.prefs);
	const quality = $derived(p.quality === 'auto' ? `Automatic, data saver ${p.saverKbps} kb/s on mobile data` : p.quality === 'original' ? 'Original' : `Data saver ${p.saverKbps} kb/s`);
	const theme = $derived(`${THEMES.find((t) => t.id === appearance.prefs.theme)?.name ?? ''}, ${appearance.prefs.mode === 'auto' ? 'follows the system' : appearance.prefs.mode}`);
</script>

<main class="page narrow">
	<h1>Settings</h1>
	{#if session.user}
		<header class="who">
			<span class="initial serif" aria-hidden="true">{(session.user.display_name || session.user.username).slice(0, 1).toUpperCase()}</span>
			<span class="name">
				<span class="serif dn">{session.user.display_name || session.user.username}</span>
				<span class="muted">{session.user.username}{session.user.role === 'admin' ? ' · admin' : ''}</span>
			</span>
		</header>
	{/if}
	<nav class="rules list" aria-label="Settings">
		<SettingsRow href="#/settings/playback" icon="play" title="Playback" summary="{quality} · skip {p.skipBack} s / {p.skipForward} s · {p.speed}×" />
		<SettingsRow href="#/settings/appearance" icon="palette" title="Appearance" summary={theme} />
		<SettingsRow href="#/settings/account" icon="user" title="Account" summary="{session.user?.display_name ?? ''} · passkeys, two-step, recovery codes" />
		<SettingsRow href="#/settings/devices" icon="device" title="Devices" summary="Where you’re signed in" />
		<SettingsRow href="#/settings/history" icon="clock" title="History" summary="Your listening time and recent sessions" />
		<SettingsRow href="#/settings/about" icon="info" title="About" summary="Version, privacy and storage" />
		{#if session.user?.role === 'admin'}
			<SettingsRow href="#/admin" icon="key" title="Admin" summary="People, invites, library scans, audit log and security" />
		{/if}
	</nav>
</main>

<style>
	.who {
		display: flex;
		align-items: center;
		gap: 1rem;
		margin: 0.5rem 0 1.5rem;
	}
	.initial {
		display: grid;
		place-items: center;
		width: 56px;
		height: 56px;
		flex: none;
		border-radius: 50%;
		background: var(--fg);
		color: var(--bg);
		font-size: 1.6rem;
		font-weight: 600;
	}
	.name {
		display: grid;
		min-width: 0;
		font-size: 0.9rem;
	}
	.dn {
		font-size: 1.3rem;
		font-weight: 600;
	}
	.list {
		border-top: 1px solid var(--line);
		border-bottom: 1px solid var(--line);
	}
</style>
