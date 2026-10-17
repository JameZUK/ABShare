<script lang="ts">
	// Authors or narrators as links to their pages: "A, B and C", or the first three and
	// "and N others" for a full cast.
	import { personHref, type PersonKind } from '$lib/people';

	let { people, kind }: { people: ({ name: string } | string)[]; kind: PersonKind } = $props();
	const list = $derived(people.map((p) => (typeof p === 'string' ? p : p.name)).filter(Boolean));
	const shown = $derived(list.length > 4 ? list.slice(0, 3) : list);
	const others = $derived(list.length - shown.length);
</script>

{#each shown as name, i (name + i)}{#if i > 0}{i === shown.length - 1 && !others ? ' and ' : ', '}{/if}<a href={personHref(kind, name)}>{name}</a>{/each}{#if others} and {others} others{/if}
