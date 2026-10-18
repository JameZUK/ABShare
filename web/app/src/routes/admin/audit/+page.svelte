<script lang="ts">
	import { adminApi, type AuditEntry } from '$lib/api/admin';
	import { messageOf } from '$lib/api/http';
	import { session } from '$lib/api/session.svelte';
	import AdminShell from '$lib/components/AdminShell.svelte';
	import Icon from '$lib/components/Icon.svelte';

	let entries = $state<AuditEntry[]>([]);
	let next = $state<number | null>(null);
	let chain = $state<{ intact: boolean; entries?: number; broken_at?: number } | null>(null);
	let error = $state('');

	async function load(more = false) {
		try {
			const r = await adminApi.audit(more ? (next ?? undefined) : undefined);
			entries = more ? [...entries, ...r.entries] : r.entries;
			next = r.next;
			chain = r.chain;
		} catch (e) {
			error = messageOf(e);
		}
	}
	$effect(() => {
		if (session.status === 'signed-in' && session.user?.role === 'admin') void load();
	});
	const when = (s: number) => new Date(s * 1000).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
</script>

<AdminShell title="Audit log">
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#if chain}
		<p class:bad={!chain.intact} class="chain">
			<Icon name={chain.intact ? 'check' : 'warning'} size={18} />
			{chain.intact ? `The log is intact: ${chain.entries} entries, none edited or removed.` : `The log has been tampered with at entry ${chain.broken_at}.`}
		</p>
	{/if}
	<div class="scroll"><table>
		<thead><tr><th>When</th><th>Who</th><th>What</th><th>Details</th></tr></thead>
		<tbody>
			{#each entries as e (e.id)}
				<tr>
					<td class="nowrap">{when(e.at)}</td>
					<td>{e.user ?? '—'}</td>
					<td class="nowrap">{e.event}</td>
					<td class="muted">{e.detail}{e.ip ? ` · ${e.ip}` : ''}</td>
				</tr>
			{/each}
		</tbody>
	</table></div>
	{#if next}<button class="btn soft more" onclick={() => load(true)}>Older entries</button>{/if}
</AdminShell>

<style>
	.chain {
		display: flex;
		gap: 0.6rem;
		align-items: center;
		padding: 0.8rem 1rem;
		border-radius: var(--radius-sm);
		border: 1px solid var(--line);
	}
	.chain.bad {
		color: var(--danger);
		border-color: var(--danger);
		font-weight: 600;
	}
	.scroll {
		overflow-x: auto;
	}
	table {
		width: 100%;
		border-collapse: collapse;
		font-size: 0.875rem;
	}
	th {
		color: var(--fg-muted);
		font-size: 0.72rem;
		font-weight: 600;
		letter-spacing: 0.12em;
		text-transform: uppercase;
		border-bottom: 1px solid var(--fg);
	}
	th,
	td {
		text-align: left;
		padding: 0.6rem 0.6rem 0.6rem 0;
		vertical-align: top;
	}
	td {
		border-bottom: 1px solid var(--line);
	}
	td:nth-child(3) {
		font-weight: 600;
	}
	.nowrap {
		white-space: nowrap;
		font-variant-numeric: tabular-nums;
	}
	.more {
		margin-top: 1.25rem;
	}
	@media (max-width: 600px) {
		th:nth-child(4),
		td:nth-child(4) {
			display: none;
		}
	}
</style>
