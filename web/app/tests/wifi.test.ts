// "Download only on Wi-Fi": held only when asked for and the browser reports mobile data.
import { shouldHold, typeKnown } from '$lib/offline/wifi';
import { DEFAULTS, sanitise } from '$lib/settings.svelte';

test('downloads are held only on reported mobile data, and only when Wi-Fi only is on', () => {
	expect(shouldHold(true, { type: 'cellular' })).toBe(true);
	expect(shouldHold(true, { type: 'wifi', saveData: true })).toBe(true);
	expect(shouldHold(true, { type: 'wifi' })).toBe(false);
	expect(shouldHold(true, { type: 'ethernet' })).toBe(false);
	expect(shouldHold(false, { type: 'cellular' })).toBe(false);
	// Nothing reported (Safari, Firefox): never held.
	expect(shouldHold(true, undefined)).toBe(false);
	expect(shouldHold(true, {})).toBe(false);
});

test('whether the browser reports the connection type', () => {
	expect(typeKnown({ type: 'wifi' })).toBe(true);
	expect(typeKnown({ type: 'cellular' })).toBe(true);
	expect(typeKnown({ type: 'unknown' })).toBe(false);
	expect(typeKnown({})).toBe(false);
	expect(typeKnown(undefined)).toBe(false);
});

test('the Wi-Fi only preference is checked like the others', () => {
	expect(DEFAULTS.wifiOnly).toBe(false);
	expect(sanitise({}).wifiOnly).toBe(false);
	expect(sanitise({ wifiOnly: true }).wifiOnly).toBe(true);
	for (const bad of ['true', 1, null, {}, []]) expect(sanitise({ wifiOnly: bad }).wifiOnly).toBe(false);
	// Choices saved before it existed keep everything else.
	expect(sanitise({ downloadQuality: 'saver', skipBack: 30 })).toMatchObject({ downloadQuality: 'saver', skipBack: 30, wifiOnly: false });
});
