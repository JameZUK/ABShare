<script lang="ts">
	import { adminApi, type LibraryView } from '$lib/api/admin';
	import { messageOf } from '$lib/api/http';
	import { session } from '$lib/api/session.svelte';
	import { date } from '$lib/format';
	import AdminShell from '$lib/components/AdminShell.svelte';
	import { withStepUp } from '$lib/stepup.svelte';

	let v = $state<LibraryView | null>(null);
	let error = $state('');
	let showInfo = $state(false);
	async function reload() {
		try {
			v = await adminApi.library();
		} catch (e) {
			error = messageOf(e);
		}
	}
	$effect(() => {
		if (session.status === 'signed-in' && session.user?.role === 'admin') void reload();
	});
	async function scan() {
		try {
			await withStepUp(() => adminApi.scan());
			await reload();
		} catch (e) {
			error = messageOf(e);
		}
	}
</script>

<AdminShell title="Library">
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#if v}
		<div class="row scanrow">
			<p class="spacer muted">
				{#if v.last_scan}Last scanned {date(v.last_scan.at)} {new Date(v.last_scan.at * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}, taking {v.last_scan.took_seconds} s.{:else}No scan recorded yet.{/if}
			</p>
			<button class="btn soft" onclick={scan} disabled={v.scan_requested}>{v.scan_requested ? 'Scan requested' : 'Scan now'}</button>
		</div>
		{#each v.libraries as l (l.id)}
			<div class="card lib">
				<h3 class="serif">{l.name}</h3>
				<p class="muted">{l.books} books · {l.groups} series and shows · {l.covers} covers · {l.needs_copy} need a playable copy</p>
			</div>
		{/each}

		<h2>Needs attention ({v.problems.attention.length})</h2>
		{#each v.problems.attention as p (p.library + p.path + p.kind)}
			<div class="problem"><strong>{p.kind}</strong> <span class="path">{p.path}</span><br /><span class="muted">{p.detail}</span></div>
		{:else}
			<p class="muted">Nothing to fix.</p>
		{/each}

		<h2>
			<button class="link" onclick={() => (showInfo = !showInfo)} aria-expanded={showInfo}>
				Informational ({v.problems.info.length}) <span class="caret" class:open={showInfo} aria-hidden="true">▸</span>
			</button>
		</h2>
		<p class="muted small">Duplicate files are ignored: one copy of each book is used. They can be tidied up in the library whenever convenient.</p>
		{#if showInfo}
			{#each v.problems.info as p (p.library + p.path + p.kind)}
				<div class="problem"><strong>{p.kind}</strong> <span class="path">{p.path}</span><br /><span class="muted">{p.detail}</span></div>
			{/each}
		{/if}
	{/if}
</AdminShell>

<style>
	.scanrow {
		flex-wrap: wrap;
		margin-bottom: 1.25rem;
	}
	.lib {
		margin-bottom: 1rem;
	}
	.lib h3 {
		font-size: 1.25rem;
		margin-bottom: 0.25rem;
	}
	.lib p {
		margin: 0;
	}
	h2 {
		margin: 2rem 0 0.75rem;
		padding-top: 1.25rem;
		border-top: 1px solid var(--line);
	}
	.problem {
		padding: 0.75rem 0;
		border-bottom: 1px solid var(--line);
		font-size: 0.9rem;
		word-break: break-word;
	}
	.problem strong {
		display: inline-block;
		margin-right: 0.35rem;
		color: var(--accent);
		font-size: 0.72rem;
		letter-spacing: 0.1em;
		text-transform: uppercase;
	}
	.path {
		font-family: ui-monospace, monospace;
		font-size: 0.82rem;
	}
	.link {
		display: inline-flex;
		align-items: center;
		gap: 0.5rem;
		border: 0;
		background: none;
		font: inherit;
		color: inherit;
		padding: 0;
		min-height: 44px;
	}
	.caret {
		display: inline-block;
		font-size: 0.8em;
		color: var(--fg-muted);
		transition: transform 0.15s var(--ease);
	}
	.caret.open {
		transform: rotate(90deg);
	}
	.small {
		font-size: 0.85rem;
	}
</style>
