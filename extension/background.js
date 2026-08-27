// The service worker does the actual talking to the desktop app. Content scripts
// can be flaky reaching localhost, so they hand the state here and we forward it.
const ENDPOINT = 'http://127.0.0.1:41414/state';

async function forward(payload) {
	try {
		const res = await fetch(ENDPOINT, {
			method: 'POST',
			headers: { 'Content-Type': 'application/json' },
			body: JSON.stringify(payload)
		});
		return { ok: res.ok, status: res.status };
	} catch (err) {
		return { ok: false, error: String(err) };
	}
}

chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
	if (msg?.type !== 'yt-rpc-state') return false;
	forward(msg.payload).then(result => {
		if (!result.ok) console.warn('[yt-rpc bg] bridge unreachable:', result);
		sendResponse(result);
	});
	return true;
});

// First install: show the setup page so people know they also need the desktop app.
chrome.runtime.onInstalled.addListener(details => {
	if (details.reason === 'install') {
		chrome.tabs.create({ url: chrome.runtime.getURL('welcome.html') });
	}
});
