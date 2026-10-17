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

<main class="page">
	<h1>Settings</h1>
	<SettingsRow href="#/settings/playback" icon="play" title="Playback" summary="{quality} · skip {p.skipBack} s / {p.skipForward} s · {p.speed}×" />
	<SettingsRow href="#/settings/appearance" icon="palette" title="Appearance" summary={theme} />
	<SettingsRow href="#/settings/account" icon="user" title="Account" summary="{session.user?.display_name ?? ''} · passkeys, two-step, recovery codes" />
	<SettingsRow href="#/settings/devices" icon="device" title="Devices" summary="Where you’re signed in" />
	<SettingsRow href="#/settings/about" icon="info" title="About" summary="Version, privacy and storage" />
</main>
