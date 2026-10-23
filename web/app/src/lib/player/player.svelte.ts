// The player (SPEC §7.3). One <audio> element, created once and reused for every book: iOS
// ties background playback to the element. Positions are in ms throughout.
import { api, session } from '$lib/api/session.svelte';
import type { BookDetail, Progress } from '$lib/api/types';
import { downloads } from '$lib/offline/downloads.svelte';
import { sync } from '$lib/offline/sync.svelte';
import { settings, type SaverKbps } from '$lib/settings.svelte';
import { chapterAt, nextTarget, previousTarget } from './chapters';
import { autoRewindMs, chooseQuality, clampStart, fadeVolume, streamUrl } from './rules';
import { swContainer } from '$lib/pwa/update.svelte';

export type Source = 'original' | SaverKbps | 'device';
export type Sleep = { kind: 'off' } | { kind: 'time'; remainingMs: number } | { kind: 'chapter' };

const LAST_BOOK = 'abshare.lastBook';
const REPORT_EVERY_MS = 15_000;

function remember(id: string | null) {
	try {
		if (id) localStorage.setItem(LAST_BOOK, id);
		else localStorage.removeItem(LAST_BOOK);
	} catch {
		/* not essential */
	}
}

export function lastBook(): string | null {
	try {
		const v = localStorage.getItem(LAST_BOOK);
		return v && /^[a-z2-7]{26}$/.test(v) ? v : null;
	} catch {
		return null;
	}
}

/** How close to the end counts as finished: 30 s, or 2% of a short item. */
export const finishMargin = (durationMs: number) => Math.min(30_000, Math.floor(durationMs / 50));

/** Picks where to start: the newer of the server's and this device's positions. */
export function startPosition(server: Progress | null, local: { position_ms: number; at: number } | undefined, durationMs: number): number {
	let pos = 0;
	if (server && (!local || server.updated_at >= local.at)) pos = server.finished ? 0 : server.position_ms;
	else if (local) pos = local.position_ms;
	// Near the very end counts as finished: start again (the server's rule, core::progress).
	return pos >= durationMs - finishMargin(durationMs) ? 0 : Math.max(0, pos);
}

class Player {
	book = $state<BookDetail | null>(null);
	position = $state(0);
	duration = $state(0);
	playing = $state(false);
	buffering = $state(false);
	source = $state<Source>('original');
	speed = $state(1);
	sleep = $state<Sleep>({ kind: 'off' });
	/** Shown in the status slot ("Tap play to start", "Preparing this book…"). */
	notice = $state<string | null>(null);
	preparing = $state(false);
	/** The full-screen player is open. */
	expanded = $state(false);
	/** A book just finished on this device, for whoever wants to know (the "How was it?"
	 *  prompt). Set by the end of playback and by marking a book finished; the listener clears it. */
	finishedNow = $state<{ id: string; title: string; cover: string | null } | null>(null);

	#audio: HTMLAudioElement | null = null;
	#pendingSeek: number | null = null;
	#lastReport = 0;
	#lastTick = 0;
	#pausedAt = 0;
	#retried = 0;
	#coverUrl: string | null = null;
	#prepareTimer: ReturnType<typeof setTimeout> | undefined;

	get chapter(): number {
		return this.book ? chapterAt(this.book.chapters, this.position) : -1;
	}

	/** Creates the audio element. Call once at start-up. */
	start() {
		if (this.#audio || typeof document === 'undefined') return;
		const a = document.createElement('audio');
		a.preload = 'auto';
		a.setAttribute('playsinline', '');
		a.style.display = 'none';
		document.body.append(a);
		this.#audio = a;
		a.addEventListener('loadedmetadata', () => {
			if (this.#pendingSeek !== null) {
				a.currentTime = this.#pendingSeek / 1000;
				this.#pendingSeek = null;
			}
			this.#retried = 0;
		});
		a.addEventListener('timeupdate', () => this.#tick());
		a.addEventListener('playing', () => {
			this.playing = true;
			this.buffering = false;
			this.notice = null;
			this.#lastTick = performance.now();
		});
		a.addEventListener('pause', () => {
			this.playing = false;
			this.#pausedAt = Date.now();
			this.#report(true);
		});
		a.addEventListener('waiting', () => (this.buffering = true));
		a.addEventListener('seeked', () => {
			this.buffering = false;
			this.position = a.currentTime * 1000;
			this.#lastReport = 0;
		});
		a.addEventListener('ended', () => {
			this.playing = false;
			if (this.book) {
				void sync.record(this.book.id, 'finished');
				this.finishedNow = { id: this.book.id, title: this.book.title, cover: this.book.cover };
			}
			this.#clearSleep();
		});
		a.addEventListener('error', () => void this.#onError());
		document.addEventListener('visibilitychange', () => {
			if (document.visibilityState === 'hidden') this.#report(true);
			else void this.checkElsewhere();
		});
		addEventListener('pagehide', () => this.#report(true));
		this.#mediaSession();
	}

	/** Opens a book, at its resume position. Plays only when asked (a user's tap). */
	async open(detail: BookDetail, opts: { play?: boolean; at?: number } = {}) {
		const a = this.#audio;
		if (!a) return;
		if (this.book && this.book.id !== detail.id) this.#report(true);
		this.book = detail;
		this.duration = detail.duration_ms;
		this.speed = settings.speedFor(detail.id);
		this.notice = null;
		remember(detail.id);
		const local = await sync.local(detail.id);
		const at = opts.at === undefined ? startPosition(detail.progress, local, detail.duration_ms) : clampStart(opts.at, detail.duration_ms);
		this.position = at;
		void this.#artwork(detail);
		await this.#load(at, opts.play ?? false);
	}

	async #load(at: number, play: boolean) {
		const a = this.#audio!;
		const book = this.book!;
		clearTimeout(this.#prepareTimer);
		this.preparing = false;
		let src: string;
		if (downloads.playable(book.id) && session.user && swContainer()?.controller) {
			this.source = 'device';
			src = `offline/${session.user.username}/${book.id}`;
		} else {
			// Still connecting on a slow connection: wait for the answer rather than give up.
			if (session.status === 'connecting') {
				this.notice = 'Connecting… The book starts when the server answers.';
				await session.settled();
				if (this.book !== book) return;
				this.notice = null;
			}
			if (session.status !== 'signed-in') {
				this.notice = 'This book isn’t downloaded. Connect to listen.';
				return;
			}
			const ready = book.media.preparing ? await this.#ready(book.id) : true;
			if (ready === null) {
				this.notice = 'This book couldn’t be prepared for playback. The library may have a damaged file.';
				return;
			}
			if (!ready) {
				this.preparing = true;
				this.notice = 'Preparing this book for playback. It starts by itself in a moment.';
				this.#prepareTimer = setTimeout(() => void this.#load(at, play), 5_000);
				return;
			}
			const conn = (navigator as Navigator & { connection?: { type?: string; effectiveType?: string; saveData?: boolean } }).connection;
			const q = chooseQuality(settings.prefs.quality, settings.prefs.saverKbps, conn);
			this.source = q;
			src = streamUrl(book.id, q);
		}
		this.#pendingSeek = at;
		a.src = src;
		a.playbackRate = this.speed;
		a.preservesPitch = true;
		a.load();
		if (play) await this.play();
	}

	/** Whether the book's playable copy is ready; null when building it failed. */
	async #ready(id: string): Promise<boolean | null> {
		try {
			const r = await api<{ ready: boolean; failed?: boolean }>(`v1/media/${encodeURIComponent(id)}/state`);
			return r.failed ? null : r.ready;
		} catch {
			return false;
		}
	}

	async play() {
		const a = this.#audio;
		if (!a || !this.book || this.preparing) return;
		if (!a.src) await this.#load(this.position, false);
		if (settings.prefs.autoRewind && this.#pausedAt && a.readyState > 0) {
			const back = autoRewindMs(Date.now() - this.#pausedAt);
			if (back > 0) a.currentTime = Math.max(0, a.currentTime - back / 1000);
		}
		this.#pausedAt = 0;
		// Listening again to a finished book: it's in progress again.
		const b = this.book;
		if (b.progress?.finished) {
			void sync.record(b.id, 'restart');
			this.book = { ...b, progress: { ...b.progress, finished: false, position_ms: 0 } };
		}
		try {
			await a.play();
			this.notice = null;
		} catch (e) {
			if (e instanceof DOMException && e.name === 'NotAllowedError') this.notice = 'Tap play to start listening.';
			else if (e instanceof DOMException && e.name === 'AbortError' && this.#retried++ < 1) await this.play();
		}
	}

	pause() {
		this.#audio?.pause();
	}

	toggle() {
		if (this.playing) this.pause();
		else void this.play();
	}

	/** Seeks to a position in the book (ms). One seek per user action. */
	seek(ms: number) {
		const a = this.#audio;
		if (!a || !this.book) return;
		const limit = (Number.isFinite(a.duration) && a.duration > 0 ? a.duration * 1000 : this.duration) - 500;
		const t = Math.max(0, Math.min(ms, limit));
		this.position = t;
		if (a.readyState === 0) this.#pendingSeek = t;
		else a.currentTime = t / 1000;
	}

	skip(deltaSec: number) {
		this.seek(this.position + deltaSec * 1000);
	}

	previousChapter() {
		if (this.book) this.seek(previousTarget(this.book.chapters, this.position));
	}

	nextChapter() {
		const t = this.book ? nextTarget(this.book.chapters, this.position) : null;
		if (t !== null) this.seek(t);
	}

	setSpeed(s: number) {
		this.speed = s;
		if (this.#audio) this.#audio.playbackRate = s;
		if (this.book) settings.setSpeedFor(this.book.id, s);
	}

	setSleep(s: Sleep) {
		this.sleep = s;
		if (this.#audio) this.#audio.volume = 1;
	}

	#clearSleep() {
		this.sleep = { kind: 'off' };
		if (this.#audio) this.#audio.volume = 1;
	}

	/** Changes streaming quality at the same position. */
	async setSource(q: 'original' | SaverKbps) {
		if (!this.book) return;
		settings.set(q === 'original' ? { quality: 'original' } : { quality: 'saver', saverKbps: q });
		await this.#load(this.position, this.playing);
	}

	/** Marks the open book finished or not, or starts it again. */
	async mark(action: 'finished' | 'unfinished' | 'restart') {
		if (!this.book) return;
		await sync.record(this.book.id, action);
		if (action === 'restart') this.seek(0);
		if (action === 'finished') this.finishedNow = { id: this.book.id, title: this.book.title, cover: this.book.cover };
	}

	/** Stops and forgets the open book (sign-out). */
	close() {
		const a = this.#audio;
		this.#report(true);
		if (a) {
			a.pause();
			a.removeAttribute('src');
			a.load();
		}
		this.book = null;
		this.expanded = false;
		remember(null);
	}

	/** After coming back to the app: if another device moved on, follow it (when paused). */
	async checkElsewhere() {
		const b = this.book;
		if (!b || this.playing || session.status !== 'signed-in') return;
		try {
			const fresh = await api<BookDetail>(`v1/books/${b.id}`);
			const local = await sync.local(b.id);
			if (fresh.progress && (!local || fresh.progress.updated_at > local.at) && Math.abs(fresh.progress.position_ms - this.position) > 2_000) {
				this.book = { ...b, progress: fresh.progress };
				this.seek(fresh.progress.finished ? 0 : fresh.progress.position_ms);
			}
		} catch {
			/* offline: keep the local position */
		}
	}

	#tick() {
		const a = this.#audio!;
		const now = performance.now();
		const delta = this.#lastTick ? now - this.#lastTick : 0;
		this.#lastTick = now;
		this.position = a.currentTime * 1000;
		if (Number.isFinite(a.duration) && a.duration > 0 && this.source !== 'device') this.duration = Math.max(this.duration, a.duration * 1000);
		// The sleep timer runs from timeupdate events, which keep coming in the background.
		const s = this.sleep;
		if (!a.paused && s.kind === 'time') {
			const remainingMs = s.remainingMs - delta;
			a.volume = fadeVolume(remainingMs);
			if (remainingMs <= 0) {
				a.pause();
				this.#clearSleep();
			} else this.sleep = { kind: 'time', remainingMs };
		} else if (!a.paused && s.kind === 'chapter' && this.book) {
			const i = this.chapter;
			const end = i >= 0 && i + 1 < this.book.chapters.length ? this.book.chapters[i + 1]!.start_ms : this.duration;
			a.volume = fadeVolume(end - this.position);
			if (this.position >= end - 250) {
				a.pause();
				this.#clearSleep();
			}
		}
		if (!a.paused && Date.now() - this.#lastReport > REPORT_EVERY_MS) this.#report(false);
		this.#positionState();
	}

	#report(force: boolean) {
		const b = this.book;
		const a = this.#audio;
		if (!b || !a || a.readyState === 0) return;
		if (!force && Date.now() - this.#lastReport < REPORT_EVERY_MS) return;
		this.#lastReport = Date.now();
		void sync.record(b.id, 'position', a.currentTime * 1000);
	}

	async #onError() {
		const a = this.#audio!;
		if (!this.book || !a.src) return;
		const pos = a.currentTime > 0 ? a.currentTime * 1000 : this.position;
		// A lapsed media cookie or a dropped connection: renew and pick up where it stopped.
		if (this.#retried < 3 && this.source !== 'device') {
			this.#retried++;
			const ok = await session.refresh();
			if (ok) {
				setTimeout(() => void this.#load(pos, this.playing || !a.paused), 1_000 * this.#retried);
				return;
			}
		}
		this.playing = false;
		this.notice = session.status === 'offline' ? 'You’re offline. Downloaded books still play.' : 'This book couldn’t be played. Try again in a moment.';
	}

	async #artwork(detail: BookDetail) {
		if (this.#coverUrl) URL.revokeObjectURL(this.#coverUrl);
		this.#coverUrl = null;
		let blob: Blob | undefined = await downloads.cover(detail.id).catch(() => undefined);
		if (!blob && detail.cover && session.status === 'signed-in') {
			try {
				const r = await fetch(`v1/${detail.cover}?size=640`, { credentials: 'same-origin' });
				if (r.ok) blob = await r.blob();
			} catch {
				/* no artwork */
			}
		}
		if (blob) this.#coverUrl = URL.createObjectURL(blob);
		this.#metadata();
	}

	#metadata() {
		if (!('mediaSession' in navigator) || !this.book) return;
		const b = this.book;
		const ch = this.chapter >= 0 ? b.chapters[this.chapter]?.title : undefined;
		navigator.mediaSession.metadata = new MediaMetadata({
			title: b.title,
			artist: ch ?? (b.authors ?? []).map((p) => p.name).join(', '),
			album: (b.authors ?? []).map((p) => p.name).join(', '),
			artwork: this.#coverUrl ? [{ src: this.#coverUrl, sizes: '640x640', type: 'image/jpeg' }] : [],
		});
	}

	#lastChapter = -2;
	#positionState() {
		if (!('mediaSession' in navigator)) return;
		if (this.chapter !== this.#lastChapter) {
			this.#lastChapter = this.chapter;
			this.#metadata();
		}
		const a = this.#audio!;
		if (Number.isFinite(a.duration) && a.duration > 0) {
			try {
				navigator.mediaSession.setPositionState({ duration: a.duration, position: Math.min(a.currentTime, a.duration), playbackRate: a.playbackRate });
			} catch {
				/* unsupported */
			}
		}
	}

	#mediaSession() {
		if (!('mediaSession' in navigator)) return;
		const set = (action: MediaSessionAction, h: MediaSessionActionHandler) => {
			try {
				navigator.mediaSession.setActionHandler(action, h);
			} catch {
				/* not supported here */
			}
		};
		set('play', () => void this.play());
		set('pause', () => this.pause());
		set('seekbackward', (d) => this.skip(-(d.seekOffset ?? settings.prefs.skipBack)));
		set('seekforward', (d) => this.skip(d.seekOffset ?? settings.prefs.skipForward));
		set('seekto', (d) => {
			if (d.seekTime !== undefined && !d.fastSeek) this.seek(d.seekTime * 1000);
		});
		set('previoustrack', () => this.previousChapter());
		set('nexttrack', () => this.nextChapter());
	}
}

export const player = new Player();

