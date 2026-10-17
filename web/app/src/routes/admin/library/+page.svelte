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
		<div class="row">
			<p class="spacer muted">
				{#if v.last_scan}Last scanned {date(v.last_scan.at)} {new Date(v.last_scan.at * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}, taking {v.last_scan.took_seconds} s.{:else}No scan recorded yet.{/if}
			</p>
			<button class="btn soft" onclick={scan} disabled={v.scan_requested}>{v.scan_requested ? 'Scan requested' : 'Scan now'}</button>
		</div>
		{#each v.libraries as l (l.id)}
			<div class="card lib">
				<h2>{l.name}</h2>
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
				Informational ({v.problems.info.length}) {showInfo ? '▾' : '▸'}
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
	.lib {
		margin-bottom: 1rem;
	}
	.problem {
		padding: 0.5rem 0;
		border-bottom: 1px solid var(--line);
		font-size: 0.9rem;
		word-break: break-word;
	}
	.path {
		font-family: ui-monospace, monospace;
		font-size: 0.85rem;
	}
	.link {
		border: 0;
		background: none;
		font: inherit;
		color: inherit;
		padding: 0;
		min-height: 44px;
	}
	.small {
		font-size: 0.85rem;
	}
</style>
