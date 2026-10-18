<script lang="ts">
	// Server settings (D35): what the config file sets, and the rules built into this version.
	// Read-only on purpose: changing them means editing abshare.toml and restarting, so a
	// stolen admin session can't change how the server behaves.
	import { adminApi, type ServerSettings } from '$lib/api/admin';
	import { messageOf } from '$lib/api/http';
	import { session } from '$lib/api/session.svelte';
	import AdminShell from '$lib/components/AdminShell.svelte';

	let v = $state<ServerSettings | null>(null);
	let error = $state('');
	$effect(() => {
		if (session.status !== 'signed-in' || session.user?.role !== 'admin') return;
		adminApi.settings().then((s) => (v = s)).catch((e) => (error = messageOf(e)));
	});
	const count = (n: number, one: string, many = one + 's') => `${n.toLocaleString('en-GB')} ${n === 1 ? one : many}`;
</script>

<AdminShell title="Server settings">
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#if v}
		<p class="muted lede">These come from the server’s config file, <code>abshare.toml</code>. To change one, edit the file and restart ABShare. They can’t be changed here.</p>

		<h2>Config file</h2>
		<dl class="rows">
			<div><dt>Address</dt><dd>{v.config.public_origin}</dd></div>
			<div><dt>Listens on</dt><dd>{v.config.listen}</dd></div>
			<div>
				<dt>Trusted proxies</dt>
				<dd>
					{#if v.config.trusted_proxies.length}{v.config.trusted_proxies.join(', ')}{:else}<span class="muted">None: client addresses come from the connection</span>{/if}
				</dd>
			</div>
			<div><dt>Libraries</dt><dd>{v.config.libraries.join(', ')}</dd></div>
			<div><dt>ffmpeg jobs at once</dt><dd>{v.config.ffmpeg_jobs}</dd></div>
			<div><dt>Transcode cache limit</dt><dd>{v.config.cache_max_gib} GiB</dd></div>
			<div><dt>Web app</dt><dd>{v.config.web_app ? 'Served by ABShare' : 'Not served (API only)'}</dd></div>
		</dl>

		<h2>Built-in rules</h2>
		<p class="muted">Fixed in this version of ABShare.</p>
		<dl class="rows">
			<div><dt>Two-step sign-in</dt><dd>Always required (a passkey, or a password with an authenticator app)</dd></div>
			<div><dt>Signed in on a device</dt><dd>Up to {count(v.rules.sign_in_days, 'day')}, then sign in again</dd></div>
			<div><dt>Devices per person</dt><dd>Up to {v.rules.max_devices}</dd></div>
			<div><dt>“Confirm it’s you” lasts</dt><dd>{count(v.rules.step_up_minutes, 'minute')}</dd></div>
			<div><dt>Invite links</dt><dd>Single use, valid for {count(v.rules.invite_hours, 'hour')}</dd></div>
			<div><dt>Listening history kept</dt><dd>{count(v.rules.history_days, 'day')}</dd></div>
			<div><dt>A book counts as abandoned</dt><dd>After {count(v.rules.abandoned_days, 'day')} without listening</dd></div>
			<div><dt>Audit log kept</dt><dd>{count(v.rules.audit_days, 'day')}, at most {count(v.rules.audit_rows, 'entry', 'entries')}</dd></div>
		</dl>
	{/if}
</AdminShell>

<style>
	.lede {
		margin-top: 0;
	}
	h2 {
		margin: 2rem 0 0.75rem;
		padding-top: 1.25rem;
		border-top: 1px solid var(--line);
	}
	.rows {
		margin: 0;
	}
	.rows div {
		display: flex;
		flex-wrap: wrap;
		gap: 0.25rem 1rem;
		padding: 0.7rem 0;
		border-bottom: 1px solid var(--line);
	}
	dt {
		flex: 0 0 13rem;
		color: var(--fg-muted);
		font-weight: 600;
	}
	dd {
		flex: 1 1 14rem;
		margin: 0;
		overflow-wrap: anywhere;
	}
</style>
