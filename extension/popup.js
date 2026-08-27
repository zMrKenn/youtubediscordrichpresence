// The popup: quick status (is the desktop app up?) plus a couple of links.
// Change PROJECT_URL to wherever you host the desktop app download.
const PROJECT_URL = 'https://keen.pub';
const PORT = 41414;

function setStatus(state) {
	const dot = document.getElementById('dot');
	const txt = document.getElementById('statusText');
	const hint = document.getElementById('hint');
	if (state === 'up') {
		dot.className = 'dot up';
		txt.textContent = 'Desktop app is running';
		hint.textContent = 'Play something on YouTube and it shows up on your Discord.';
	} else if (state === 'down') {
		dot.className = 'dot down';
		txt.textContent = 'Desktop app not running';
		hint.textContent = "Open the desktop app - the red play icon in your tray. Grab it below if you don't have it yet.";
	} else {
		dot.className = 'dot';
		txt.textContent = 'Checking...';
		hint.textContent = '';
	}
}

async function check() {
	setStatus('checking');
	const ctrl = new AbortController();
	const timer = setTimeout(() => ctrl.abort(), 1500);
	try {
		const res = await fetch(`http://127.0.0.1:${PORT}/ping`, { signal: ctrl.signal });
		clearTimeout(timer);
		setStatus(res.ok ? 'up' : 'down');
	} catch {
		clearTimeout(timer);
		setStatus('down');
	}
}

document.getElementById('getApp').addEventListener('click', () => {
	chrome.tabs.create({ url: PROJECT_URL });
});
document.getElementById('guide').addEventListener('click', () => {
	chrome.tabs.create({ url: chrome.runtime.getURL('welcome.html') });
});

check();
