<script lang="ts">
	// Everyone's listening at a glance: time heard, books finished, in progress and abandoned
	// (started, not finished, nothing heard for 30 days), and when they last listened.
	import { adminApi, type PersonListening } from '$lib/api/admin';
	import { messageOf } from '$lib/api/http';
	import { session } from '$lib/api/session.svelte';
	import { ago } from '$lib/when';
	import AdminShell from '$lib/components/AdminShell.svelte';
	import Icon from '$lib/components/Icon.svelte';

	let people = $state<PersonListening[] | null>(null);
	let error = $state('');
	$effect(() => {
		if (session.status !== 'signed-in' || session.user?.role !== 'admin') return;
		adminApi
			.listening()
			.then((p) => (people = [...p].sort((a, b) => (b.last_listened_at ?? 0) - (a.last_listened_at ?? 0))))
			.catch((e) => (error = messageOf(e)));
	});
	const hours = (s: number) => (s < 3600 ? `${Math.round(s / 60)} min` : `${(s / 3600).toFixed(s < 36000 ? 1 : 0)} h`);
</script>

<AdminShell title="Listening">
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	{#if people === null && !error}<p class="muted">Loading…</p>{/if}
	{#if people}
		<p class="muted lead">Time heard counts only listening while someone’s history is on. Abandoned means started, not finished, and nothing heard for 30 days.</p>
		<ul class="people">
			{#each people as p (p.id)}
				<li>
					<a href="#/admin/listening/{p.id}">
						<span class="who">
							<span class="serif name">{p.name}</span>
							<span class="muted small">{p.username}{p.disabled ? ' · disabled' : ''}{p.history_on ? '' : ' · history off'} · {p.last_listened_at ? `listened ${ago(p.last_listened_at)}` : 'not listened yet'}</span>
						</span>
						<span class="figures">
							<span><b>{hours(p.totals.month_seconds)}</b><small>30 days</small></span>
							<span><b>{hours(p.totals.all_seconds)}</b><small>in all</small></span>
							<span><b>{p.totals.finished}</b><small>finished</small></span>
							<span><b>{p.totals.in_progress}</b><small>listening</small></span>
							<span><b>{p.totals.abandoned}</b><small>abandoned</small></span>
						</span>
						<Icon name="right" size={18} />
					</a>
				</li>
			{/each}
		</ul>
	{/if}
</AdminShell>

<style>
	.lead {
		font-size: 0.875rem;
		max-width: 640px;
	}
	.people {
		list-style: none;
		margin: 1rem 0 0;
		padding: 0;
		border-top: 1px solid var(--line);
	}
	.people li {
		border-bottom: 1px solid var(--line);
	}
	.people a {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.6rem 1.25rem;
		padding: 0.9rem 0;
		color: inherit;
		text-decoration: none;
	}
	.people a > :global(svg) {
		color: var(--fg-muted);
		margin-left: auto;
	}
	.who {
		display: grid;
		flex: 1 1 220px;
		min-width: 0;
	}
	.name {
		font-size: 1.1rem;
		font-weight: 600;
	}
	.small {
		font-size: 0.82rem;
	}
	.figures {
		display: grid;
		grid-template-columns: repeat(5, minmax(0, auto));
		gap: 1.1rem;
	}
	@media (max-width: 559px) {
		.figures {
			order: 3;
			width: 100%;
			grid-template-columns: repeat(5, minmax(0, 1fr));
			gap: 0.5rem;
		}
	}
	.figures span {
		display: grid;
		text-align: right;
		white-space: nowrap;
	}
	.figures b {
		font-family: var(--font-display);
		font-size: 1.1rem;
		font-variant-numeric: tabular-nums;
	}
	.figures small {
		font-size: 0.7rem;
		letter-spacing: 0.06em;
		text-transform: uppercase;
		color: var(--fg-muted);
	}
</style>
