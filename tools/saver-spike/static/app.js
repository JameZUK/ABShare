// Phase 0(f) test page: seeking, background playback and quality switching.
// Text only: nothing here builds HTML from strings.
'use strict';

const audio = document.getElementById('player');
const log = document.getElementById('log');
const status = document.getElementById('status');
const ua = navigator.userAgent;
let seekStarted = 0;

function fmt(t) {
  if (!isFinite(t)) return '–';
  const h = Math.floor(t / 3600), m = Math.floor(t / 60) % 60, s = Math.floor(t % 60);
  return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
}

function note(msg) {
  const li = document.createElement('li');
  li.textContent = `${new Date().toISOString().slice(11, 23)} ${msg}`;
  log.prepend(li);
  while (log.children.length > 200) log.lastChild.remove();
  fetch('log', { method: 'POST', body: `${ua} :: ${msg}`, keepalive: true }).catch(() => {});
}

function source() {
  return document.querySelector('input[name=src]:checked').value;
}

function setSource(keepTime) {
  const t = keepTime ? audio.currentTime : 0;
  const wasPlaying = !audio.paused;
  audio.src = source();
  note(`source = ${source()} at ${fmt(t)}`);
  if (t > 0) audio.addEventListener('loadedmetadata', () => { audio.currentTime = t; }, { once: true });
  if (wasPlaying) audio.play().catch((e) => note(`play refused: ${e.name}`));
}

function seekTo(t) {
  seekStarted = performance.now();
  audio.currentTime = Math.max(0, Math.min(t, (audio.duration || 0) - 1));
}

for (const ev of ['loadedmetadata', 'playing', 'pause', 'waiting', 'stalled', 'ended', 'error']) {
  audio.addEventListener(ev, () => {
    const extra = ev === 'error' && audio.error ? ` code ${audio.error.code}` : '';
    const dur = ev === 'loadedmetadata' ? ` duration ${fmt(audio.duration)}` : '';
    note(`${ev} @ ${fmt(audio.currentTime)}${dur}${extra}`);
  });
}
audio.addEventListener('seeking', () => { if (!seekStarted) seekStarted = performance.now(); note(`seeking → ${fmt(audio.currentTime)}`); });
audio.addEventListener('seeked', () => {
  note(`seeked @ ${fmt(audio.currentTime)} in ${Math.round(performance.now() - seekStarted)} ms`);
  seekStarted = 0;
});
document.addEventListener('visibilitychange', () => note(`page ${document.visibilityState} @ ${fmt(audio.currentTime)}`));

setInterval(() => {
  const b = audio.buffered;
  const ranges = [];
  for (let i = 0; i < b.length; i++) ranges.push(`${fmt(b.start(i))}–${fmt(b.end(i))}`);
  status.textContent = `${fmt(audio.currentTime)} / ${fmt(audio.duration)} · ${audio.paused ? 'paused' : 'playing'} · buffered ${ranges.join(', ') || 'none'}`;
}, 500);

document.getElementById('play').addEventListener('click', () => {
  if (!audio.src) setSource(false);
  if (audio.paused) audio.play().catch((e) => note(`play refused: ${e.name}`)); else audio.pause();
});
document.getElementById('back').addEventListener('click', () => seekTo(audio.currentTime - 30));
document.getElementById('fwd').addEventListener('click', () => seekTo(audio.currentTime + 30));
document.getElementById('random').addEventListener('click', () => seekTo(Math.random() * (audio.duration || 0)));
for (const b of document.querySelectorAll('button[data-frac]')) {
  b.addEventListener('click', () => seekTo(Number(b.dataset.frac) * (audio.duration || 0)));
}
for (const r of document.querySelectorAll('input[name=src]')) r.addEventListener('change', () => setSource(true));

if ('mediaSession' in navigator) {
  navigator.mediaSession.metadata = new MediaMetadata({ title: 'Data saver spike', artist: 'ABShare' });
  const set = (a, h) => { try { navigator.mediaSession.setActionHandler(a, h); } catch { /* unsupported */ } };
  set('play', () => audio.play());
  set('pause', () => audio.pause());
  set('seekbackward', () => seekTo(audio.currentTime - 30));
  set('seekforward', () => seekTo(audio.currentTime + 30));
  set('seekto', (d) => seekTo(d.seekTime));
  setInterval(() => {
    if (isFinite(audio.duration) && audio.duration > 0) {
      try {
        navigator.mediaSession.setPositionState({ duration: audio.duration, position: Math.min(audio.currentTime, audio.duration), playbackRate: audio.playbackRate });
      } catch { /* ignore */ }
    }
  }, 2000);
}

audio.src = source();
note(`page loaded: ${ua}`);
