<script lang="ts">
	// The frame of every admin page: admins only, the admin tabs, and the step-up sheet that
	// account changes ask for.
	import type { Snippet } from 'svelte';
	import { api, session } from '$lib/api/session.svelte';
	import type { Me } from '$lib/api/types';
	import AdminGate from './AdminGate.svelte';
	import AdminNav from './AdminNav.svelte';
	import StepUpSheet from './StepUpSheet.svelte';

	let { title, children }: { title: string; children: Snippet } = $props();
	let me = $state<Me | null>(null);
	$effect(() => {
		if (session.status === 'signed-in') api<Me>('v1/me').then((m) => (me = m)).catch(() => {});
	});
</script>

<AdminGate>
	<main class="page">
		<AdminNav {title} />
		{@render children()}
	</main>
	<StepUpSheet {me} />
</AdminGate>
