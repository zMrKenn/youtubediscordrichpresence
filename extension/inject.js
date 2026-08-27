// runs in the page (MAIN world) to read YouTube's player API and pass the current
// video to the content script. the URL lies on Music/playlists; the player doesn't.
(() => {
	const EMIT_MS = 1000;

	function getPlayer() {
		const byId = document.getElementById('movie_player');
		if (byId && typeof byId.getVideoData === 'function') return byId;
		const els = document.querySelectorAll('.html5-video-player, ytmusic-player, #movie_player');
		for (const el of els) {
			if (el && typeof el.getVideoData === 'function') return el;
		}
		return null;
	}

	function collect() {
		const p = getPlayer();
		if (!p) return null;
		let data, state, cur, dur, url;
		try {
			data = p.getVideoData();
			state = p.getPlayerState();
			cur = p.getCurrentTime();
			dur = p.getDuration();
			url = p.getVideoUrl();
		} catch {
			return null;
		}
		if (!data || !data.video_id) return null;
		return {
			videoId: data.video_id,
			title: (data.title || '').trim(),
			author: (data.author || '').trim(),
			playerState: state,
			currentTime: Math.floor(cur || 0),
			duration: Math.floor(dur || 0),
			url: url || ''
		};
	}

	function emit() {
		const d = collect();
		if (d) {
			try { window.dispatchEvent(new CustomEvent('YTRPC_PLAYER', { detail: d })); } catch {}
		}
	}

	setInterval(emit, EMIT_MS);
	emit();
})();
