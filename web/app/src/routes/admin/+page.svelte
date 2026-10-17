<script lang="ts">
	import { adminApi, type DiagReport, type Status } from '$lib/api/admin';
	import { messageOf } from '$lib/api/http';
	import { session } from '$lib/api/session.svelte';
	import { bytes } from '$lib/format';
	import AdminShell from '$lib/components/AdminShell.svelte';
	import Icon from '$lib/components/Icon.svelte';

	let st = $state<Status | null>(null);
	let reports = $state<DiagReport[]>([]);
	let error = $state('');
	$effect(() => {
		if (session.status !== 'signed-in' || session.user?.role !== 'admin') return;
		adminApi.status().then((s) => (st = s)).catch((e) => (error = messageOf(e)));
		adminApi.diagnostics().then((r) => (reports = r)).catch(() => (reports = []));
	});
	const when = (at: number) => new Date(at * 1000).toLocaleString('en-GB', { dateStyle: 'medium', timeStyle: 'short' });
</script>

<AdminShell title="Admin">
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#if st}
		<section class="warnings">
			{#each st.warnings as w (w)}
				<p class="warn"><Icon name="warning" size={18} /> {w}</p>
			{:else}
				<p class="ok"><Icon name="check" size={18} /> No security warnings.</p>
			{/each}
		</section>
		<dl class="stats">
			<div><dt>People</dt><dd>{st.users}</dd></div>
			<div><dt>Admins</dt><dd>{st.admins}</dd></div>
			<div><dt>Disabled</dt><dd>{st.disabled}</dd></div>
			<div><dt>Open invites</dt><dd>{st.pending_invites}</dd></div>
			<div><dt>Two-step sign-in</dt><dd>{Math.round(st.two_step_coverage * 100)}%</dd></div>
			<div><dt>ffmpeg sandbox</dt><dd>{st.sandbox.landlock ? 'On' : 'Off'}</dd></div>
			<div><dt>Transcode cache</dt><dd>{bytes(st.cache_bytes)}</dd></div>
			<div><dt>Version</dt><dd>{st.version}</dd></div>
		</dl>
	{/if}
	{#if reports.length}
		<section class="reports">
			<h2>Start-up reports ({reports.length})</h2>
			<p class="muted">Sent by browsers that were left with a blank page. Kept in memory until the server restarts.</p>
			{#each reports as r (r.at + r.addr + r.lines.length)}
				<details>
					<summary>{when(r.at)} · {r.addr}</summary>
					<pre>{r.lines.join('\n')}</pre>
				</details>
			{/each}
		</section>
	{/if}
</AdminShell>

<style>
	.warnings p {
		display: flex;
		gap: 0.5rem;
		align-items: flex-start;
		padding: 0.6rem 0.8rem;
		border-radius: var(--radius-sm);
		margin: 0 0 0.5rem;
	}
	.warn {
		background: var(--accent-soft);
		color: var(--accent-soft-fg);
	}
	.ok {
		background: var(--bg-sunken);
	}
	.stats {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
		gap: 0.75rem;
		margin: 1rem 0 0;
	}
	.stats div {
		background: var(--bg-raised);
		box-shadow: var(--shadow);
		border-radius: var(--radius-sm);
		padding: 0.75rem 1rem;
	}
	dt {
		color: var(--fg-muted);
		font-size: 0.85rem;
	}
	.reports {
		margin-top: 1.5rem;
	}
	.reports summary {
		cursor: pointer;
		padding: 0.5rem 0;
		min-height: 44px;
	}
	.reports pre {
		white-space: pre-wrap;
		word-break: break-word;
		background: var(--bg-sunken);
		padding: 0.75rem;
		border-radius: var(--radius-sm);
		font-size: 0.85rem;
	}
	dd {
		margin: 0.2rem 0 0;
		font-size: 1.3rem;
		font-weight: 700;
	}
</style>
