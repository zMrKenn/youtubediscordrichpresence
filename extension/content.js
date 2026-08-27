// Picks up what inject.js reads from the player, tidies it up (title, channel,
// album, progress) and forwards it to the background worker every couple seconds.
// Only sends when something actually changed, plus a heartbeat so the app knows
// we're still here.
(() => {
	const POLL_MS = 2000;
	const HEARTBEAT_MS = 12000;
	const READY_RETRY_MS = 500;
	const READY_MAX_TRIES = 8;
	const PLAYER_STALE_MS = 6000;

	let last = null;
	let lastSentAt = 0;
	let readyRetries = 0;
	let pollHandle = null;
	let dead = false;
	let bridgeDown = false;

	let player = null;
	let playerAt = 0;

	window.addEventListener('YTRPC_PLAYER', (e) => {
		if (e?.detail?.videoId) {
			player = e.detail;
			playerAt = Date.now();
		}
	});

	function die(reason) {
		if (dead) return;
		dead = true;
		if (pollHandle) clearInterval(pollHandle);
		// Not an error - just the old script bowing out after an extension reload.
		console.log(`[yt-rpc] stopped: ${reason}. Refresh this tab to reconnect.`);
	}

	function isMusicSite() {
		return location.hostname === 'music.youtube.com';
	}

	function musicByline() {
		const byline = document.querySelector('ytmusic-player-bar .content-info-wrapper .byline')
			|| document.querySelector('ytmusic-player-bar .byline');
		if (!byline) return { artist: '', album: '', artistUrl: '' };
		const links = Array.from(byline.querySelectorAll('a'));
		const texts = links.map(a => a.textContent.trim()).filter(Boolean);
		return {
			artist: texts[0] || '',
			album: texts[1] || '',
			artistUrl: links[0]?.href || ''
		};
	}

	function regularChannelEl() {
		return document.querySelector('ytd-watch-metadata ytd-channel-name #text a')
			|| document.querySelector('ytd-channel-name#channel-name #text a')
			|| document.querySelector('ytd-channel-name #text a')
			|| document.querySelector('ytd-video-owner-renderer #channel-name a')
			|| document.querySelector('#owner #channel-name a');
	}

	function pickChannelUrl() {
		if (isMusicSite()) return musicByline().artistUrl;
		return regularChannelEl()?.href || '';
	}

	function pickAlbum() {
		if (!isMusicSite()) return '';
		const album = musicByline().album;
		if (!album || /^\d{4}$/.test(album)) return '';
		return album;
	}

	function pickState() {
		if (!player || Date.now() - playerAt > PLAYER_STALE_MS) return null;
		const p = player;
		if (!p.videoId) return null;

		const state = p.playerState;
		const ended = state === 0;
		const playing = state === 1 || state === 3;
		const live = (!p.duration || p.duration === 0) && (state === 1 || state === 3);

		const channel = (isMusicSite() ? musicByline().artist : '') || p.author || '';
		const album = pickAlbum();
		const isMusic = isMusicSite() || /\s-\sTopic$/i.test(p.author || '');

		return {
			videoId: p.videoId,
			url: isMusicSite()
				? `https://music.youtube.com/watch?v=${p.videoId}`
				: `https://www.youtube.com/watch?v=${p.videoId}`,
			channelUrl: pickChannelUrl(),
			title: p.title || '',
			channel,
			album,
			live,
			ended,
			currentTime: p.currentTime,
			duration: p.duration,
			playing,
			isMusic,
			thumbnail: `https://i.ytimg.com/vi/${p.videoId}/hqdefault.jpg`
		};
	}

	function isReady(state) {
		if (!state || !state.title) return false;
		if (state.ended) return false;
		return state.live || state.duration > 0;
	}

	function shouldSend(state) {
		if (!last) return true;
		if (last.videoId !== state.videoId) return true;
		if (last.playing !== state.playing) return true;
		if (last.isMusic !== state.isMusic) return true;
		if (last.live !== state.live) return true;
		if (last.title !== state.title) return true;
		if (last.channel !== state.channel) return true;
		if (last.album !== state.album) return true;
		const expected = (Date.now() - lastSentAt) / 1000;
		const actual = state.currentTime - last.currentTime;
		if (Math.abs(actual - expected) > 3) return true;
		if (Date.now() - lastSentAt > HEARTBEAT_MS) return true;
		return false;
	}

	function send(payload) {
		if (dead) return;
		let hasContext = false;
		try { hasContext = !!chrome.runtime?.id; } catch {}
		if (!hasContext) return die('extension reloaded');
		try {
			chrome.runtime.sendMessage({ type: 'yt-rpc-state', payload }, (result) => {
				const err = chrome.runtime.lastError;
				if (err) return die(err.message);
				if (result?.ok) {
					lastSentAt = Date.now();
					last = payload.clear ? null : payload;
					if (bridgeDown) { console.log('[yt-rpc] bridge reconnected'); bridgeDown = false; }
				} else if (!bridgeDown) {
					console.log('[yt-rpc] bridge not running — start the app; retrying quietly');
					bridgeDown = true;
				}
			});
		} catch (err) {
			die(err?.message || String(err));
		}
	}

	function tick() {
		if (dead) return;
		const state = pickState();
		if (!state) return;
		if (!isReady(state)) {
			if (readyRetries < READY_MAX_TRIES) {
				readyRetries += 1;
				setTimeout(tick, READY_RETRY_MS);
			}
			return;
		}
		readyRetries = 0;
		if (!shouldSend(state)) return;
		console.log('[yt-rpc] →', state.isMusic ? '🎵' : '📺', state.live ? 'LIVE' : (state.playing ? '▶' : '⏸'), state.title, state.channel ? `— ${state.channel}` : '(no channel)', state.live ? '' : `${state.currentTime}s / ${state.duration}s`);
		send(state);
	}

	pollHandle = setInterval(tick, POLL_MS);
	tick();
})();
