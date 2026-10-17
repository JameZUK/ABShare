// Phase 0(e): WebAuthn registration and sign-in against webauthn-rs.
// Converts between base64url JSON (server) and ArrayBuffers (browser) by hand, so it works
// in browsers without PublicKeyCredential.parseCreationOptionsFromJSON / toJSON.
'use strict';

const logEl = document.getElementById('log');
const resultEl = document.getElementById('result');
const ua = navigator.userAgent;

function note(msg) {
  const li = document.createElement('li');
  li.textContent = `${new Date().toISOString().slice(11, 23)} ${msg}`;
  logEl.prepend(li);
  fetch('log', { method: 'POST', body: `${ua} :: passkey :: ${msg}`, keepalive: true }).catch(() => {});
}

function b64uToBuf(s) {
  const pad = '='.repeat((4 - (s.length % 4)) % 4);
  const bin = atob((s + pad).replace(/-/g, '+').replace(/_/g, '/'));
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out.buffer;
}

function bufToB64u(buf) {
  const bytes = new Uint8Array(buf);
  let bin = '';
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

async function post(path, body) {
  const resp = await fetch(path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
  const text = await resp.text();
  if (!resp.ok) throw new Error(`${path}: ${resp.status} ${text}`);
  return JSON.parse(text);
}

async function register() {
  try {
    const { id, options } = await post('pk/register/start', { name: document.getElementById('name').value });
    const pk = options.publicKey;
    pk.challenge = b64uToBuf(pk.challenge);
    pk.user.id = b64uToBuf(pk.user.id);
    for (const c of pk.excludeCredentials || []) c.id = b64uToBuf(c.id);
    note(`create(): rp ${pk.rp.id}, userVerification ${pk.authenticatorSelection && pk.authenticatorSelection.userVerification}`);
    const cred = await navigator.credentials.create({ publicKey: pk });
    const credential = {
      id: cred.id,
      rawId: bufToB64u(cred.rawId),
      type: cred.type,
      extensions: cred.getClientExtensionResults ? cred.getClientExtensionResults() : {},
      response: {
        attestationObject: bufToB64u(cred.response.attestationObject),
        clientDataJSON: bufToB64u(cred.response.clientDataJSON),
        transports: cred.response.getTransports ? cred.response.getTransports() : undefined,
      },
    };
    const done = await post('pk/register/finish', { id, credential });
    resultEl.textContent = done.detail;
    note(done.detail);
  } catch (e) {
    resultEl.textContent = `Failed: ${e.name || ''} ${e.message || e}`;
    note(resultEl.textContent);
  }
}

async function login() {
  try {
    const { id, options } = await post('pk/login/start', {});
    const pk = options.publicKey;
    pk.challenge = b64uToBuf(pk.challenge);
    for (const c of pk.allowCredentials || []) c.id = b64uToBuf(c.id);
    const cred = await navigator.credentials.get({ publicKey: pk });
    const credential = {
      id: cred.id,
      rawId: bufToB64u(cred.rawId),
      type: cred.type,
      extensions: cred.getClientExtensionResults ? cred.getClientExtensionResults() : {},
      response: {
        authenticatorData: bufToB64u(cred.response.authenticatorData),
        clientDataJSON: bufToB64u(cred.response.clientDataJSON),
        signature: bufToB64u(cred.response.signature),
        userHandle: cred.response.userHandle ? bufToB64u(cred.response.userHandle) : null,
      },
    };
    const done = await post('pk/login/finish', { id, credential });
    resultEl.textContent = done.detail;
    note(done.detail);
  } catch (e) {
    resultEl.textContent = `Failed: ${e.name || ''} ${e.message || e}`;
    note(resultEl.textContent);
  }
}

document.getElementById('register').addEventListener('click', register);
document.getElementById('login').addEventListener('click', login);
note(`passkey page loaded: ${ua}; WebAuthn ${window.PublicKeyCredential ? 'available' : 'MISSING'}`);
