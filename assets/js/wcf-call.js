// Thin bridge between the Leptos call page and livekit-client.
//
// The Rust side calls `window.WcfCall.*`; this file owns the media grid
// (video/audio elements) inside the container element and reports state
// changes back via `window` events named "wcf-call" whose `detail` is a
// plain JSON object: { type: "...", ... }.
(function () {
  "use strict";

  const LIB_URL = "/vendor/livekit-client.umd.js";
  let libPromise = null;
  let room = null;
  let container = null;
  let localIdentity = null;

  function emit(detail) {
    window.dispatchEvent(new CustomEvent("wcf-call", { detail: JSON.stringify(detail) }));
  }

  function loadLib() {
    if (window.LivekitClient) return Promise.resolve(window.LivekitClient);
    if (!libPromise) {
      libPromise = new Promise((resolve, reject) => {
        const s = document.createElement("script");
        s.src = LIB_URL;
        s.async = true;
        s.onload = () => resolve(window.LivekitClient);
        s.onerror = () => {
          libPromise = null;
          reject(new Error("LiveKit-Bibliothek konnte nicht geladen werden."));
        };
        document.head.appendChild(s);
      });
    }
    return libPromise;
  }

  function initials(name) {
    const parts = (name || "?").trim().split(/\s+/).filter(Boolean);
    const letters = parts.length > 1 ? parts[0][0] + parts[parts.length - 1][0] : (parts[0] || "?").slice(0, 2);
    return letters.toUpperCase();
  }

  function hue(id) {
    let h = 0;
    for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) % 360;
    return h;
  }

  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }

  // --- tiles -----------------------------------------------------------

  function tileId(identity, screen) {
    return "call-tile-" + (screen ? "screen-" : "") + identity.replace(/[^a-zA-Z0-9_-]/g, "");
  }

  function ensureTile(p, screen) {
    if (!container) return null;
    const id = tileId(p.identity, screen);
    let tile = document.getElementById(id);
    if (!tile) {
      tile = el("div", "call-tile" + (screen ? " call-tile-screen" : ""));
      tile.id = id;
      tile.dataset.identity = p.identity;
      const avatar = el("div", "call-avatar", initials(p.name || p.identity));
      avatar.style.setProperty("--hue", hue(p.identity));
      const video = el("video");
      video.autoplay = true;
      video.playsInline = true;
      video.muted = true; // audio is attached to separate elements
      const label = el("div", "call-label");
      const name = el("span", "call-name", (p.name || p.identity) + (screen ? " – Bildschirm" : ""));
      const mic = el("span", "call-mic");
      label.appendChild(mic);
      label.appendChild(name);
      tile.appendChild(avatar);
      tile.appendChild(video);
      tile.appendChild(label);
      if (screen) {
        const full = el("button", "call-fullscreen", "⛶");
        full.title = "Vollbild";
        full.onclick = () => (tile.requestFullscreen ? tile.requestFullscreen() : null);
        tile.appendChild(full);
        container.prepend(tile);
      } else if (p.identity === localIdentity) {
        tile.classList.add("is-local");
        container.appendChild(tile);
      } else {
        container.appendChild(tile);
      }
    }
    return tile;
  }

  function removeTile(identity, screen) {
    const t = document.getElementById(tileId(identity, screen));
    if (t) t.remove();
  }

  function refreshTile(p) {
    const L = window.LivekitClient;
    const tile = ensureTile(p, false);
    if (!tile) return;
    const camPub = p.getTrackPublication(L.Track.Source.Camera);
    const video = tile.querySelector("video");
    const camOn = !!(camPub && camPub.track && !camPub.isMuted);
    if (camOn) {
      if (video.dataset.sid !== camPub.trackSid) {
        camPub.track.attach(video);
        video.dataset.sid = camPub.trackSid;
      }
    } else if (video.dataset.sid) {
      video.srcObject = null;
      delete video.dataset.sid;
    }
    tile.classList.toggle("has-video", camOn);
    tile.classList.toggle("is-local-mirror", camOn && p.identity === localIdentity);
    tile.classList.toggle("speaking", !!p.isSpeaking);
    tile.querySelector(".call-mic").textContent = p.isMicrophoneEnabled ? "" : "🔇";

    const screenPub = p.getTrackPublication(L.Track.Source.ScreenShare);
    if (screenPub && screenPub.track && !screenPub.isMuted) {
      const st = ensureTile(p, true);
      const sv = st.querySelector("video");
      if (sv.dataset.sid !== screenPub.trackSid) {
        screenPub.track.attach(sv);
        sv.dataset.sid = screenPub.trackSid;
      }
      st.classList.add("has-video");
    } else {
      removeTile(p.identity, true);
    }
    if (container) {
      container.classList.toggle("has-screen", !!container.querySelector(".call-tile-screen"));
      container.dataset.count = String(container.querySelectorAll(".call-tile").length);
    }
  }

  function allParticipants() {
    if (!room) return [];
    return [room.localParticipant, ...room.remoteParticipants.values()];
  }

  function report() {
    const list = allParticipants().map((p) => ({
      identity: p.identity,
      name: p.name || p.identity,
      speaking: !!p.isSpeaking,
      mic: !!p.isMicrophoneEnabled,
      cam: !!p.isCameraEnabled,
      screen: !!p.isScreenShareEnabled,
      local: p.identity === localIdentity,
    }));
    emit({ type: "participants", participants: list });
    if (room) {
      const lp = room.localParticipant;
      emit({
        type: "local",
        mic: !!lp.isMicrophoneEnabled,
        cam: !!lp.isCameraEnabled,
        screen: !!lp.isScreenShareEnabled,
      });
    }
  }

  function refreshAll() {
    allParticipants().forEach(refreshTile);
    report();
  }

  function attachAudio(track, participant) {
    const L = window.LivekitClient;
    if (track.kind !== L.Track.Kind.Audio) return;
    const a = track.attach();
    a.dataset.identity = participant.identity;
    a.className = "call-audio";
    document.body.appendChild(a);
  }

  // --- public API ------------------------------------------------------

  async function join(opts) {
    // opts: { url, token, identity, containerId, mic, cam, micDevice, camDevice }
    const L = await loadLib();
    if (room) await leave();
    container = document.getElementById(opts.containerId);
    localIdentity = opts.identity;
    room = new L.Room({
      adaptiveStream: true,
      dynacast: true,
      audioCaptureDefaults: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
      videoCaptureDefaults: { resolution: L.VideoPresets.h720.resolution },
    });
    const E = L.RoomEvent;
    room
      .on(E.ParticipantConnected, (p) => { refreshTile(p); report(); })
      .on(E.ParticipantDisconnected, (p) => { removeTile(p.identity, false); removeTile(p.identity, true); report(); })
      .on(E.TrackSubscribed, (track, _pub, p) => { attachAudio(track, p); refreshTile(p); report(); })
      .on(E.TrackUnsubscribed, (track, _pub, p) => { track.detach().forEach((e) => e.remove()); refreshTile(p); report(); })
      .on(E.TrackMuted, (_pub, p) => { refreshTile(p); report(); })
      .on(E.TrackUnmuted, (_pub, p) => { refreshTile(p); report(); })
      .on(E.LocalTrackPublished, () => refreshAll())
      .on(E.LocalTrackUnpublished, () => refreshAll())
      .on(E.ActiveSpeakersChanged, () => refreshAll())
      .on(E.MediaDevicesChanged, () => devices().then((d) => emit({ type: "devices", devices: d })))
      .on(E.AudioPlaybackStatusChanged, () => emit({ type: "audio", blocked: !room.canPlaybackAudio }))
      .on(E.Reconnecting, () => emit({ type: "state", state: "reconnecting" }))
      .on(E.Reconnected, () => emit({ type: "state", state: "connected" }))
      .on(E.Disconnected, (reason) => {
        emit({ type: "state", state: "disconnected", reason: String(reason ?? "") });
        cleanup();
      });

    emit({ type: "state", state: "connecting" });
    try {
      await room.connect(opts.url, opts.token);
    } catch (e) {
      emit({
        type: "state",
        state: "failed",
        error: "Keine Verbindung zum Medienserver (" + String((e && e.message) || e) + ").",
      });
      cleanup();
      return;
    }
    emit({ type: "state", state: "connected" });
    refreshAll();
    try {
      if (opts.micDevice) await room.switchActiveDevice("audioinput", opts.micDevice);
      if (opts.camDevice) await room.switchActiveDevice("videoinput", opts.camDevice);
    } catch (_) {}
    try {
      await room.localParticipant.setMicrophoneEnabled(!!opts.mic);
    } catch (e) {
      emit({ type: "error", message: "Mikrofon nicht verfügbar: " + ((e && e.message) || e) });
    }
    try {
      if (opts.cam) await room.localParticipant.setCameraEnabled(true);
    } catch (e) {
      emit({ type: "error", message: "Kamera nicht verfügbar: " + ((e && e.message) || e) });
    }
    emit({ type: "audio", blocked: !room.canPlaybackAudio });
    refreshAll();
  }

  function cleanup() {
    document.querySelectorAll("audio.call-audio").forEach((a) => a.remove());
    if (container) {
      container.innerHTML = "";
      container.classList.remove("has-screen");
    }
    room = null;
    emit({ type: "participants", participants: [] });
  }

  async function leave() {
    if (room) {
      const r = room;
      room = null;
      try {
        await r.disconnect();
      } catch (_) {}
    }
    cleanup();
  }

  async function toggle(kind) {
    if (!room) return;
    const lp = room.localParticipant;
    try {
      if (kind === "mic") await lp.setMicrophoneEnabled(!lp.isMicrophoneEnabled);
      else if (kind === "cam") await lp.setCameraEnabled(!lp.isCameraEnabled);
      else if (kind === "screen") await lp.setScreenShareEnabled(!lp.isScreenShareEnabled, { audio: true });
    } catch (e) {
      if (!(e && e.name === "NotAllowedError" && kind === "screen")) {
        emit({ type: "error", message: String((e && e.message) || e) });
      }
    }
    refreshAll();
  }

  async function devices() {
    const L = await loadLib();
    const pick = (list) => list.map((d) => ({ id: d.deviceId, label: d.label || "Gerät" }));
    try {
      return {
        audioinput: pick(await L.Room.getLocalDevices("audioinput")),
        videoinput: pick(await L.Room.getLocalDevices("videoinput")),
        audiooutput: pick(await L.Room.getLocalDevices("audiooutput")),
      };
    } catch (_) {
      return { audioinput: [], videoinput: [], audiooutput: [] };
    }
  }

  async function setDevice(kind, id) {
    if (!room) return;
    try {
      await room.switchActiveDevice(kind, id);
    } catch (e) {
      emit({ type: "error", message: "Gerät konnte nicht gewechselt werden." });
    }
  }

  async function startAudio() {
    if (room) {
      try {
        await room.startAudio();
      } catch (_) {}
      emit({ type: "audio", blocked: !room.canPlaybackAudio });
    }
  }

  // Camera preview for the pre-join screen.
  let previewStream = null;
  async function preview(videoId, on) {
    const v = document.getElementById(videoId);
    if (previewStream) {
      previewStream.getTracks().forEach((t) => t.stop());
      previewStream = null;
    }
    if (!on || !v) {
      if (v) v.srcObject = null;
      return true;
    }
    try {
      previewStream = await navigator.mediaDevices.getUserMedia({ video: true, audio: false });
      v.srcObject = previewStream;
      return true;
    } catch (e) {
      emit({ type: "error", message: "Kamera nicht verfügbar: " + ((e && e.message) || e) });
      return false;
    }
  }

  // Simple ringtone (WebAudio) for incoming/outgoing calls.
  let ringCtx = null;
  let ringTimer = null;
  function ring(on) {
    if (ringTimer) {
      clearInterval(ringTimer);
      ringTimer = null;
    }
    if (!on) return;
    try {
      ringCtx = ringCtx || new (window.AudioContext || window.webkitAudioContext)();
      const beep = () => {
        const now = ringCtx.currentTime;
        [0, 0.25].forEach((offset) => {
          const o = ringCtx.createOscillator();
          const g = ringCtx.createGain();
          o.frequency.value = 440 + offset * 200;
          g.gain.setValueAtTime(0.0001, now + offset);
          g.gain.exponentialRampToValueAtTime(0.15, now + offset + 0.02);
          g.gain.exponentialRampToValueAtTime(0.0001, now + offset + 0.2);
          o.connect(g).connect(ringCtx.destination);
          o.start(now + offset);
          o.stop(now + offset + 0.22);
        });
      };
      beep();
      ringTimer = setInterval(beep, 2000);
    } catch (_) {}
  }

  window.WcfCall = { load: loadLib, join, leave, toggle, devices, setDevice, startAudio, preview, ring };
})();
