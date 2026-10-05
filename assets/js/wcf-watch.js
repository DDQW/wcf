// Synced watch-room player (port of HQTube's SyncedPlayer, player.ts,
// audioElection.ts and prefs.ts, extended with YouTube).
//
// The Leptos page calls `window.WcfWatch.*`; this file owns the <video> /
// YouTube iframe, hls.js, drift correction, the cross-window audio election
// and the player controls. Events go back as window "wcf-watch" events whose
// `detail` is a JSON string: { type: "duration" | "audio" | "error", ... }.
(function () {
  "use strict";

  const HLS_URL = "/vendor/hls.min.js?v=1.7.3";
  const YT_API = "https://www.youtube.com/iframe_api";
  /** |error| below this: leave playback alone. */
  const DEADBAND_MS = 150;
  /** below this: nudge playbackRate; above: hard seek. */
  const NUDGE_LIMIT_MS = 2000;
  const NUDGE_RATE = 0.05;
  const LOOP_MS = 500;
  /** YouTube only seeks reliably; looser thresholds. */
  const YT_DEADBAND_MS = 600;
  const PREFS_KEY = "wcf.player";

  function emit(detail) {
    window.dispatchEvent(new CustomEvent("wcf-watch", { detail: JSON.stringify(detail) }));
  }

  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }

  function fmt(ms) {
    const s = Math.max(0, Math.floor(ms / 1000));
    const m = Math.floor(s / 60);
    const h = Math.floor(m / 60);
    const pad = (n) => String(n).padStart(2, "0");
    return h > 0 ? `${h}:${pad(m % 60)}:${pad(s % 60)}` : `${m}:${pad(s % 60)}`;
  }

  // ---- prefs (per browser; storage may be unavailable) ----

  function loadPrefs() {
    try {
      const p = JSON.parse(localStorage.getItem(PREFS_KEY) || "{}");
      return {
        volume: typeof p.volume === "number" && p.volume >= 0 && p.volume <= 1 ? p.volume : 1,
        audioLang: typeof p.audioLang === "string" ? p.audioLang : null,
        subPick: typeof p.subPick === "string" ? p.subPick : null,
      };
    } catch (_) {
      return { volume: 1, audioLang: null, subPick: null };
    }
  }

  function savePrefs(patch) {
    try {
      localStorage.setItem(PREFS_KEY, JSON.stringify(Object.assign(loadPrefs(), patch)));
    } catch (_) {}
  }

  const norm = (l) => (l || "").toLowerCase().slice(0, 2);
  const sameLang = (a, b) => !!a && !!b && norm(a) === norm(b);
  const subPickFor = (t) => (t.lang ? t.lang : "label:" + t.label);
  const matchesSub = (t, pick) => (pick.startsWith("label:") ? t.label === pick.slice(6) : sameLang(t.lang, pick));

  // ---- lazy libraries ----

  let hlsPromise = null;
  function loadHls() {
    if (window.Hls) return Promise.resolve(window.Hls);
    if (!hlsPromise) {
      hlsPromise = new Promise((resolve, reject) => {
        const s = document.createElement("script");
        s.src = HLS_URL;
        s.onload = () => resolve(window.Hls);
        s.onerror = () => {
          hlsPromise = null;
          reject(new Error("hls.js konnte nicht geladen werden."));
        };
        document.head.appendChild(s);
      });
    }
    return hlsPromise;
  }

  let ytPromise = null;
  function loadYouTube() {
    if (window.YT && window.YT.Player) return Promise.resolve(window.YT);
    if (!ytPromise) {
      ytPromise = new Promise((resolve, reject) => {
        const prev = window.onYouTubeIframeAPIReady;
        window.onYouTubeIframeAPIReady = () => {
          if (typeof prev === "function") prev();
          resolve(window.YT);
        };
        const s = document.createElement("script");
        s.src = YT_API;
        s.onerror = () => {
          ytPromise = null;
          reject(new Error("YouTube konnte nicht geladen werden."));
        };
        document.head.appendChild(s);
      });
    }
    return ytPromise;
  }

  // ---- audio election (one sounding window per user and room) ----

  const HEARTBEAT_MS = 1500;
  const STALE_MS = 4500;
  const GRACE_MS = 300;

  function createElection(roomId, role, onChange) {
    if (typeof BroadcastChannel === "undefined") {
      onChange(true, false);
      return { claim() {}, close() {} };
    }
    const id = (crypto.randomUUID && crypto.randomUUID()) || String(Math.random());
    const bc = new BroadcastChannel("wcf-audio:" + roomId);
    let hash = 0;
    for (let i = 0; i < id.length; i++) hash = (hash * 31 + id.charCodeAt(i)) | 0;
    const jitter = Math.abs(hash) % 200;
    let mine = null;
    let known = null;
    let lastSeen = 0;
    let heartbeat = null;
    let electTimer = null;
    let graceTimer = null;
    const post = (m) => bc.postMessage(m);
    const beats = (a, b) => b === null || a.at > b.at || (a.at === b.at && a.id > b.id);
    const sync = () => onChange(mine !== null, mine === null && known !== null && known.id !== id);
    const cancelElect = () => {
      if (electTimer !== null) {
        clearTimeout(electTimer);
        electTimer = null;
      }
    };
    const yieldOwnership = () => {
      if (heartbeat !== null) clearInterval(heartbeat);
      heartbeat = null;
      mine = null;
      sync();
    };
    const claim = (at) => {
      at = at || Date.now();
      cancelElect();
      if (graceTimer !== null) {
        clearTimeout(graceTimer);
        graceTimer = null;
      }
      mine = { at };
      known = { id, at };
      lastSeen = Date.now();
      post({ t: "claim", id, at });
      if (heartbeat === null) {
        heartbeat = setInterval(() => mine && post({ t: "claim", id, at: mine.at }), HEARTBEAT_MS);
      }
      sync();
    };
    const scheduleElect = () => {
      if (mine !== null || electTimer !== null) return;
      electTimer = setTimeout(() => {
        electTimer = null;
        claim();
      }, jitter);
    };
    const checkStale = () => {
      if (mine === null && known !== null && Date.now() - lastSeen > STALE_MS) scheduleElect();
    };
    bc.onmessage = (ev) => {
      const m = ev.data;
      if (m.t === "hello") {
        if (mine !== null) post({ t: "claim", id, at: mine.at });
      } else if (m.t === "claim") {
        if (beats(m, known)) {
          known = { id: m.id, at: m.at };
          if (mine !== null) yieldOwnership();
          else sync();
        }
        if (known && known.id === m.id) lastSeen = Date.now();
        cancelElect();
      } else if (m.t === "release" && known && known.id === m.id && mine === null) {
        scheduleElect();
      }
    };
    const watchdog = setInterval(checkStale, 1000);
    const recheck = () => !document.hidden && checkStale();
    document.addEventListener("visibilitychange", recheck);
    window.addEventListener("focus", recheck);
    const onHide = () => {
      if (mine !== null) {
        post({ t: "release", id });
        yieldOwnership();
      }
    };
    window.addEventListener("pagehide", onHide);
    if (role === "popout") {
      claim();
    } else {
      post({ t: "hello", id });
      graceTimer = setTimeout(() => {
        graceTimer = null;
        if (known === null) claim();
      }, GRACE_MS);
    }
    return {
      claim: () => claim(),
      close() {
        if (mine !== null) post({ t: "release", id });
        bc.onmessage = null;
        bc.close();
        if (heartbeat !== null) clearInterval(heartbeat);
        clearInterval(watchdog);
        cancelElect();
        if (graceTimer !== null) clearTimeout(graceTimer);
        document.removeEventListener("visibilitychange", recheck);
        window.removeEventListener("focus", recheck);
        window.removeEventListener("pagehide", onHide);
      },
    };
  }

  // ---- player ----


  function isTyping(t) {
    return (
      t instanceof HTMLElement &&
      (t.isContentEditable || t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT")
    );
  }

  function createPlayer(root, opts) {
  let P = null;

  function mount() {
    const prefs = loadPrefs();
    const dom = {
      root,
      wrap: el("div", "wp-video-wrap"),
      video: el("video", "wp-video"),
      yt: el("div", "wp-yt"),
      progress: el("div", "wp-progress"),
      bar: el("div"),
      join: el("button", "wp-overlay wp-join", "🎬 Klicken, um mit Ton beizutreten"),
      elsewhere: el("button", "wp-pill", "🔊 Ton läuft in einem anderen Fenster – hier abspielen"),
      gesture: el("button", "wp-overlay wp-gesture", "🔇 Klicken für Ton"),
      controls: el("div", "wp-controls"),
      title: el("span", "wp-title"),
      time: el("span", "wp-time"),
      audioLabel: el("label", "wp-select"),
      audioSel: el("select"),
      subLabel: el("label", "wp-select"),
      subSel: el("select"),
      volWrap: el("span", "wp-volume"),
      muteBtn: el("button", "wp-btn"),
      vol: el("input", "wp-vol"),
      pipBtn: el("button", "wp-btn", "PiP"),
      fsBtn: el("button", "wp-btn", "⛶"),
    };
    dom.video.playsInline = true;
    dom.video.crossOrigin = "anonymous";
    dom.progress.appendChild(dom.bar);
    dom.wrap.append(dom.video, dom.yt, dom.progress, dom.join, dom.elsewhere, dom.gesture);
    dom.audioLabel.append("Ton ", dom.audioSel);
    dom.subLabel.append("Untertitel ", dom.subSel);
    dom.vol.type = "range";
    dom.vol.min = "0";
    dom.vol.max = "1";
    dom.vol.step = "0.05";
    dom.vol.setAttribute("aria-label", "Lautstärke");
    dom.volWrap.append(dom.muteBtn, dom.vol);
    const right = el("span", "wp-right");
    right.append(dom.volWrap);
    if (document.pictureInPictureEnabled) right.append(dom.pipBtn);
    right.append(dom.fsBtn);
    dom.controls.append(dom.title, dom.time, dom.audioLabel, dom.subLabel, right);
    const player = el("div", "wp");
    player.append(dom.wrap, dom.controls);
    root.replaceChildren(player);
    dom.player = player;
    dom.pipBtn.title = "Bild-in-Bild";
    dom.fsBtn.title = "Vollbild (F)";
    dom.audioLabel.hidden = true;
    dom.subLabel.hidden = true;

    P = {
      opts,
      dom,
      prefs,
      state: { status: "idle" },
      offset: 0,
      media: null,
      hls: null,
      yt: null,
      ytReady: false,
      subs: [],
      subIndex: -1,
      volume: prefs.volume,
      lastVolume: prefs.volume > 0 ? prefs.volume : 1,
      entered: !!(navigator.userActivation && navigator.userActivation.hasBeenActive),
      hasAudio: false,
      elsewhere: false,
      needsGesture: false,
      reportedKey: null,
      loop: null,
      tick: null,
      cleanups: [],
    };

    P.election = createElection(opts.roomId, opts.role || "main", (has, elsewhere) => {
      if (!P) return;
      P.hasAudio = has;
      P.elsewhere = elsewhere;
      emit({ type: "audio", hasAudio: has, ownerElsewhere: elsewhere });
      applyAudio();
      renderOverlays();
    });

    const onActivate = () => {
      if (P && !P.entered && (!navigator.userActivation || navigator.userActivation.hasBeenActive)) {
        P.entered = true;
        applyAudio();
        renderOverlays();
      }
    };
    document.addEventListener("pointerdown", onActivate, { capture: true });
    document.addEventListener("keydown", onActivate, { capture: true });
    P.cleanups.push(() => {
      document.removeEventListener("pointerdown", onActivate, { capture: true });
      document.removeEventListener("keydown", onActivate, { capture: true });
    });

    dom.join.onclick = () => {
      P.entered = true;
      applyAudio();
      renderOverlays();
    };
    dom.elsewhere.onclick = () => {
      P.entered = true;
      P.election.claim();
    };
    dom.gesture.onclick = () => {
      P.needsGesture = false;
      setMuted(false);
      play();
      renderOverlays();
    };
    dom.muteBtn.onclick = () => toggleMute();
    dom.vol.oninput = () => changeVolume(Number(dom.vol.value));
    dom.fsBtn.onclick = () => toggleFs();
    dom.pipBtn.onclick = () => {
      if (document.pictureInPictureElement) document.exitPictureInPicture().catch(() => {});
      else dom.video.requestPictureInPicture().catch(() => {});
    };
    dom.subSel.onchange = () => switchSub(Number(dom.subSel.value));
    dom.audioSel.onchange = () => switchAudio(Number(dom.audioSel.value));
    dom.wrap.ondblclick = () => toggleFs();
    dom.video.addEventListener("loadedmetadata", reportDuration);

    const onKey = (e) => {
      if (!P || e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey || isTyping(e.target)) return;
      const k = e.key.toLowerCase();
      if (k === "f") toggleFs();
      else if (k === "m") toggleMute();
      else if (k === "c") cycleSub();
      else return;
      e.preventDefault();
    };
    document.addEventListener("keydown", onKey);
    P.cleanups.push(() => document.removeEventListener("keydown", onKey));
    const onFs = () => (dom.fsBtn.textContent = document.fullscreenElement ? "🗗" : "⛶");
    document.addEventListener("fullscreenchange", onFs);
    P.cleanups.push(() => document.removeEventListener("fullscreenchange", onFs));

    P.loop = setInterval(correct, LOOP_MS);
    P.tick = setInterval(renderTime, 500);
    renderVolume();
    renderOverlays();
  }

  function unmount() {
    if (!P) return;
    clearInterval(P.loop);
    clearInterval(P.tick);
    teardownMedia();
    P.election.close();
    P.cleanups.forEach((f) => f());
    P.dom.root.replaceChildren();
    P = null;
  }

  const serverNow = () => Date.now() + (P ? P.offset : 0);

  function position() {
    const s = P.state;
    if (s.status === "playing") return s.position_ms + (serverNow() - s.at_server_ms) * s.rate;
    if (s.status === "paused") return s.position_ms;
    return 0;
  }

  function update(state, offset) {
    if (!P) return;
    if (typeof offset === "number" && isFinite(offset)) P.offset = offset;
    P.state = state;
    const media = state.status === "playing" || state.status === "paused" ? state.media : null;
    if (!media) {
      if (P.media) teardownMedia();
      P.dom.player.classList.remove("active");
      return;
    }
    P.dom.player.classList.add("active");
    if (!P.media || P.media.key !== media.key || P.media.url !== media.url) {
      teardownMedia();
      P.media = media;
      attachMedia(media);
    } else {
      P.media = media;
    }
    P.dom.title.textContent = media.title;
    P.dom.title.title = media.title;
    renderTime();
    correct();
  }

  function teardownMedia() {
    if (!P) return;
    if (P.hls) {
      P.hls.destroy();
      P.hls = null;
    }
    if (P.yt) {
      try {
        P.yt.destroy();
      } catch (_) {}
      P.yt = null;
      P.ytReady = false;
      P.dom.yt.replaceChildren();
    }
    const v = P.dom.video;
    v.pause();
    v.removeAttribute("src");
    v.querySelectorAll("track").forEach((t) => t.remove());
    try {
      v.load();
    } catch (_) {}
    P.media = null;
    P.subs = [];
    P.dom.audioLabel.hidden = true;
    P.dom.subLabel.hidden = true;
    P.dom.player.classList.remove("youtube");
  }

  function attachMedia(media) {
    const v = P.dom.video;
    const key = media.key;
    if (media.kind === "youtube") {
      P.dom.player.classList.add("youtube");
      const holder = el("div");
      P.dom.yt.replaceChildren(holder);
      loadYouTube()
        .then((YT) => {
          if (!P || !P.media || P.media.key !== key) return;
          P.yt = new YT.Player(holder, {
            videoId: media.url,
            playerVars: { controls: 0, disablekb: 1, modestbranding: 1, rel: 0, playsinline: 1, iv_load_policy: 3 },
            events: {
              onReady: () => {
                if (!P || !P.yt) return;
                P.ytReady = true;
                applyAudio();
                correct();
                reportDuration();
              },
              onError: () => emit({ type: "error", message: "YouTube konnte das Video nicht abspielen." }),
            },
          });
        })
        .catch((e) => emit({ type: "error", message: e.message }));
      return;
    }
    // Subtitles are side-loaded WebVTT files.
    P.subs = (media.tracks || []).filter((t) => t.kind === "sub");
    if (media.subs_base) {
      P.subs.forEach((t) => {
        const tr = el("track");
        tr.kind = "subtitles";
        tr.src = media.subs_base + "s" + t.track_index + ".vtt";
        tr.srclang = t.lang || "und";
        tr.label = t.label;
        v.appendChild(tr);
      });
    }
    renderSubs();
    const start = position() / 1000;
    if (media.kind === "hls") {
      loadHls()
        .then((Hls) => {
          if (!P || !P.media || P.media.key !== key) return;
          if (!Hls.isSupported()) {
            v.src = media.url;
            return;
          }
          const hls = new Hls(start > 0 ? { startPosition: start } : {});
          P.hls = hls;
          hls.loadSource(media.url);
          hls.attachMedia(v);
          hls.on(Hls.Events.AUDIO_TRACKS_UPDATED, () => {
            const meta = (P.media.tracks || []).filter((t) => t.kind === "audio");
            const opts = hls.audioTracks.map((t, i) => {
              const m = meta.find((a) => a.track_index === i);
              return { id: i, label: (m && m.label) || t.name || t.lang || "Ton " + (i + 1), lang: (m && m.lang) || t.lang || null };
            });
            const pref = P.prefs.audioLang ? opts.find((o) => sameLang(o.lang, P.prefs.audioLang)) : null;
            if (pref && pref.id !== hls.audioTrack) hls.audioTrack = pref.id;
            renderAudio(opts, pref ? pref.id : Math.max(0, hls.audioTrack));
          });
          hls.on(Hls.Events.ERROR, (_e, data) => {
            if (data && data.fatal) emit({ type: "error", message: "Wiedergabefehler (" + data.details + ")." });
          });
        })
        .catch(() => (v.src = media.url));
    } else {
      v.src = media.url;
    }
    applyAudio();
  }

  function reportDuration() {
    if (!P || !P.media || P.media.duration_ms != null || P.reportedKey === P.media.key) return;
    let secs = NaN;
    if (P.yt && P.ytReady) secs = P.yt.getDuration();
    else secs = P.dom.video.duration;
    if (isFinite(secs) && secs > 1) {
      P.reportedKey = P.media.key;
      emit({ type: "duration", key: P.media.key, ms: Math.round(secs * 1000) });
    }
  }

  // ---- sound ----

  const wantAudio = () => P.hasAudio && P.entered;

  function setMuted(m) {
    if (P.yt && P.ytReady) {
      if (m) P.yt.mute();
      else P.yt.unMute();
    } else {
      P.dom.video.muted = m;
    }
  }

  function play() {
    if (P.yt && P.ytReady) {
      P.yt.playVideo();
      return Promise.resolve();
    }
    return P.dom.video.play();
  }

  function applyAudio() {
    if (!P || !P.media) return;
    const want = wantAudio();
    setMuted(!want);
    applyVolume();
    if (want && P.state.status === "playing") {
      Promise.resolve(play())
        .then(() => {
          P.needsGesture = false;
          renderOverlays();
        })
        .catch(() => {
          P.needsGesture = true;
          renderOverlays();
        });
    }
  }

  function applyVolume() {
    if (P.yt && P.ytReady) P.yt.setVolume(Math.round(P.volume * 100));
    else P.dom.video.volume = P.volume;
  }

  function changeVolume(v) {
    P.volume = Math.max(0, Math.min(1, v));
    if (P.volume > 0) P.lastVolume = P.volume;
    savePrefs({ volume: P.volume });
    applyVolume();
    renderVolume();
  }

  function toggleMute() {
    changeVolume(P.volume > 0 ? 0 : P.lastVolume);
  }

  function renderVolume() {
    P.dom.vol.value = String(P.volume);
    P.dom.muteBtn.textContent = P.volume === 0 ? "🔇" : P.volume < 0.5 ? "🔉" : "🔊";
    P.dom.muteBtn.title = P.volume > 0 ? "Stumm (M)" : "Ton an (M)";
  }

  function renderOverlays() {
    if (!P) return;
    P.dom.join.hidden = !(!P.entered && P.hasAudio);
    P.dom.elsewhere.hidden = !P.elsewhere;
    P.dom.gesture.hidden = !(P.needsGesture && wantAudio());
  }

  // ---- tracks ----

  function renderAudio(opts, current) {
    const sel = P.dom.audioSel;
    sel.replaceChildren(...opts.map((o) => {
      const op = el("option", null, o.label);
      op.value = String(o.id);
      return op;
    }));
    sel.value = String(current);
    P.audioOpts = opts;
    P.dom.audioLabel.hidden = opts.length < 2;
  }

  function switchAudio(id) {
    if (P.hls) P.hls.audioTrack = id;
    const o = (P.audioOpts || []).find((x) => x.id === id);
    if (o && o.lang) {
      P.prefs.audioLang = o.lang;
      savePrefs({ audioLang: o.lang });
    }
  }

  function renderSubs() {
    const sel = P.dom.subSel;
    const off = el("option", null, "Aus");
    off.value = "-1";
    sel.replaceChildren(off, ...P.subs.map((t, i) => {
      const op = el("option", null, t.label);
      op.value = String(i);
      return op;
    }));
    const pick = P.prefs.subPick;
    const idx = pick ? P.subs.findIndex((t) => matchesSub(t, pick)) : -1;
    P.dom.subLabel.hidden = P.subs.length === 0;
    // Tracks must exist before their mode can be set.
    setTimeout(() => applySub(idx), 0);
  }

  function applySub(index) {
    if (!P) return;
    P.subIndex = index;
    P.dom.subSel.value = String(index);
    const list = P.dom.video.textTracks;
    for (let i = 0; i < list.length; i++) list[i].mode = i === index ? "showing" : "disabled";
  }

  function switchSub(index) {
    applySub(index);
    const t = P.subs[index];
    P.prefs.subPick = t ? subPickFor(t) : null;
    savePrefs({ subPick: P.prefs.subPick });
  }

  function cycleSub() {
    if (!P.subs.length) return;
    switchSub(P.subIndex + 1 >= P.subs.length ? -1 : P.subIndex + 1);
  }

  function toggleFs() {
    if (document.fullscreenElement) {
      document.exitFullscreen().catch(() => {});
    } else if (P.dom.player.requestFullscreen) {
      P.dom.player.requestFullscreen().catch(() => {});
    } else if (P.dom.video.webkitEnterFullscreen) {
      P.dom.video.webkitEnterFullscreen();
    }
  }

  function renderTime() {
    if (!P || !P.media) return;
    const dur = P.media.duration_ms;
    const pos = dur ? Math.min(position(), dur) : position();
    P.dom.time.textContent = fmt(pos) + (dur != null ? " / " + fmt(dur) : "");
    P.dom.bar.style.width = dur ? Math.min(100, (pos / dur) * 100) + "%" : "0";
  }

  // ---- drift correction ----

  function correct() {
    if (!P || !P.media) return;
    const s = P.state;
    if (s.status !== "playing" && s.status !== "paused") return;
    if (P.yt) return correctYouTube(s);
    const v = P.dom.video;
    // Hidden muted windows get paused by browsers; catch up when shown.
    if (document.hidden && v.muted) return;
    if (s.status === "paused") {
      if (!v.paused) v.pause();
      const err = s.position_ms - v.currentTime * 1000;
      if (Math.abs(err) > DEADBAND_MS && v.readyState > 0) v.currentTime = s.position_ms / 1000;
      return;
    }
    const target = position();
    if (v.paused) {
      if (v.readyState > 0) v.currentTime = target / 1000;
      tryPlay();
      return;
    }
    const err = target - v.currentTime * 1000;
    const abs = Math.abs(err);
    if (abs < DEADBAND_MS) {
      if (v.playbackRate !== s.rate) v.playbackRate = s.rate;
    } else if (abs < NUDGE_LIMIT_MS) {
      v.playbackRate = s.rate * (err > 0 ? 1 + NUDGE_RATE : 1 - NUDGE_RATE);
    } else {
      v.currentTime = (target + 100) / 1000;
      v.playbackRate = s.rate;
    }
  }

  function tryPlay() {
    const v = P.dom.video;
    if (!wantAudio()) {
      v.muted = true;
      v.play().catch(() => {});
      return;
    }
    v.play().catch(() => {
      // Autoplay policy: fall back to muted playback, ask for a click.
      v.muted = true;
      v.play()
        .then(() => {
          P.needsGesture = true;
          renderOverlays();
        })
        .catch(() => {
          P.needsGesture = true;
          renderOverlays();
        });
    });
  }

  function correctYouTube(s) {
    if (!P.ytReady) return;
    const yt = P.yt;
    const st = yt.getPlayerState();
    const cur = yt.getCurrentTime() * 1000;
    if (s.status === "paused") {
      if (st === 1 || st === 3) yt.pauseVideo();
      if (Math.abs(s.position_ms - cur) > YT_DEADBAND_MS) yt.seekTo(s.position_ms / 1000, true);
      return;
    }
    const target = position();
    if (Math.abs(target - cur) > Math.max(YT_DEADBAND_MS * 2, 1500)) yt.seekTo((target + 300) / 1000, true);
    if (st !== 1 && st !== 3) {
      if (!wantAudio()) yt.mute();
      yt.playVideo();
    }
  }


  mount();
  return {
    update,
    setOffset(ms) {
      if (P && typeof ms === "number" && isFinite(ms)) P.offset = ms;
    },
    unmount,
    claimAudio() {
      if (P) P.election.claim();
    },
  };
  }

  window.WcfWatch = {
    /** Mounts a player into `el` (element or id) and returns its handle. */
    mount(el, opts) {
      const root = typeof el === "string" ? document.getElementById(el) : el;
      if (!root) return null;
      if (root.__wcfPlayer) root.__wcfPlayer.unmount();
      const handle = createPlayer(root, opts || {});
      root.__wcfPlayer = handle;
      return handle;
    },
  };
})();
