<script lang="ts">
	// Simple line/solid icons on a 24-unit grid. Decorative unless given a label.
	const PATHS: Record<string, string> = {
		play: 'M8 5.5v13a1 1 0 0 0 1.5.86l10.4-6.5a1 1 0 0 0 0-1.72L9.5 4.64A1 1 0 0 0 8 5.5z',
		pause: 'M7 5h3.5v14H7zM13.5 5H17v14h-3.5z',
		next: 'M6 6.5v11a.8.8 0 0 0 1.25.66l7.5-5.5a.8.8 0 0 0 0-1.32l-7.5-5.5A.8.8 0 0 0 6 6.5zM16 6h2v12h-2z',
		prev: 'M18 6.5v11a.8.8 0 0 1-1.25.66l-7.5-5.5a.8.8 0 0 1 0-1.32l7.5-5.5A.8.8 0 0 1 18 6.5zM6 6h2v12H6z',
		// Skip back/forward: a curved arrow (the number is drawn beside it).
		back15: 'M4.5 12a7.5 7.5 0 1 0 2.2-5.3M4.5 4v4h4',
		fwd30: 'M19.5 12a7.5 7.5 0 1 1-2.2-5.3M19.5 4v4h-4',
		search: 'M10.5 4a6.5 6.5 0 1 1 0 13 6.5 6.5 0 0 1 0-13zM15.5 15.5 20 20',
		settings: 'M12 9a3 3 0 1 1 0 6 3 3 0 0 1 0-6zM10.26 5.01L10.42 2.53L13.58 2.53L13.74 5.01L15.71 5.83L17.57 4.18L19.82 6.43L18.17 8.29L18.99 10.26L21.47 10.42L21.47 13.58L18.99 13.74L18.17 15.71L19.82 17.57L17.57 19.82L15.71 18.17L13.74 18.99L13.58 21.47L10.42 21.47L10.26 18.99L8.29 18.17L6.43 19.82L4.18 17.57L5.83 15.71L5.01 13.74L2.53 13.58L2.53 10.42L5.01 10.26L5.83 8.29L4.18 6.43L6.43 4.18L8.29 5.83Z',
		more: 'M12 5.5h.01M12 12h.01M12 18.5h.01',
		close: 'M6 6l12 12M18 6 6 18',
		home: 'M4 11 12 4l8 7v8.5a.5.5 0 0 1-.5.5H15v-6H9v6H4.5a.5.5 0 0 1-.5-.5z',
		back: 'M15 5l-7 7 7 7',
		chevron: 'M6 9l6 6 6-6',
		right: 'M9 5l7 7-7 7',
		trash: 'M5 7h14M10 7V4.5h4V7M7 7l1 13h8l1-13',
		pencil: 'M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4',
		library: 'M5 4v16M10 4v16M14.5 4.5l4.5 15',
		download: 'M12 4v11M7.5 10.5 12 15l4.5-4.5M5 19.5h14',
		downloaded: 'M12 3a9 9 0 1 1 0 18 9 9 0 0 1 0-18zM12 7.5v8M8.5 12.5 12 16l3.5-3.5',
		check: 'M5 12.5l4.5 4.5L19 7.5',
		moon: 'M19.5 14.5A8 8 0 0 1 9.5 4.5a8 8 0 1 0 10 10z',
		list: 'M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01',
		bookmark: 'M6.5 4h11v16l-5.5-4-5.5 4z',
		speed: 'M12 20a8 8 0 1 1 8-8M12 12l4-4M12 12h.01',
		device: 'M8 2.5h8a1.5 1.5 0 0 1 1.5 1.5v16a1.5 1.5 0 0 1-1.5 1.5H8A1.5 1.5 0 0 1 6.5 20V4A1.5 1.5 0 0 1 8 2.5zM11 18.5h2',
		key: 'M14.5 4a5.5 5.5 0 1 1-4.9 8L4 17.5V20h3v-2h2v-2h2l1.1-1.1A5.5 5.5 0 0 1 14.5 4zM16 8h.01',
		offline: 'M3 8.5a13 13 0 0 1 18 0M6.5 12a8 8 0 0 1 11 0M10 15.5a3 3 0 0 1 4 0M4 4l16 16',
		info: 'M12 3a9 9 0 1 1 0 18 9 9 0 0 1 0-18zM12 11v6M12 7.5h.01',
		warning: 'M12 4 2.5 20h19zM12 10v4.5M12 17.5h.01',
		book: 'M4 5.5A1.5 1.5 0 0 1 5.5 4H11v16H5.5A1.5 1.5 0 0 1 4 18.5zM13 4h5.5A1.5 1.5 0 0 1 20 5.5v13a1.5 1.5 0 0 1-1.5 1.5H13z',
		user: 'M12 4a4 4 0 1 1 0 8 4 4 0 0 1 0-8zM4.5 20a7.5 7.5 0 0 1 15 0',
		palette: 'M12 3a9 9 0 1 0 0 18c1 0 1.5-.7 1.5-1.5 0-1.2-1-1.5-1-2.5s.8-1.5 1.8-1.5H17a4 4 0 0 0 4-4c0-4.7-4-8.5-9-8.5zM7.5 11h.01M10 7h.01M15 7.5h.01',
		signout: 'M14 4h4.5A1.5 1.5 0 0 1 20 5.5v13a1.5 1.5 0 0 1-1.5 1.5H14M10 16l-4-4 4-4M6 12h10',
		clock: 'M12 3a9 9 0 1 1 0 18 9 9 0 0 1 0-18zM12 7.5V12l3 2',
	};
	const SOLID = new Set(['play', 'pause', 'next', 'prev']);

	let { name, size = 22, label = '' }: { name: keyof typeof PATHS | string; size?: number; label?: string } = $props();
	const d = $derived(PATHS[name] ?? '');
	const solid = $derived(SOLID.has(name));
</script>

<svg
	width={size}
	height={size}
	viewBox="0 0 24 24"
	aria-hidden={label ? undefined : 'true'}
	role={label ? 'img' : undefined}
	aria-label={label || undefined}
	fill={solid ? 'currentColor' : 'none'}
	stroke={solid ? 'none' : 'currentColor'}
	stroke-width="1.8"
	stroke-linecap="round"
	stroke-linejoin="round"><path {d} /></svg
>
