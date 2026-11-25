// Download links: point each file at the newest release (previews included, which GitHub's
// /releases/latest skips), and name the visitor's own system on the big button.
// Without JavaScript, or if GitHub doesn't answer, the links open the releases page instead.
(() => {
  const REPO = 'KumaVolt/encheo';
  const FALLBACK = 'v0.6.0';
  const PRIMARY = {
    macos: ['encheo-macos-arm64.zip', 'Download for macOS'],
    windows: ['encheo-windows-x86_64-setup.exe', 'Download for Windows'],
    linux: ['encheo-linux-x86_64.tar.gz', 'Download for Linux'],
    android: ['encheo-android-arm64.apk', 'Download for Android'],
    ios: ['encheo-ios.ipa', 'Download for iPhone'],
  };

  const ua = navigator.userAgent;
  const os = /Android/i.test(ua) ? 'android'
    : /iPhone|iPad|iPod/i.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1) ? 'ios'
    : /Mac/i.test(ua) ? 'macos'
    : /Win/i.test(ua) ? 'windows'
    : /Linux|X11|CrOS/i.test(ua) ? 'linux' : null;

  const apply = (tag) => {
    document.querySelectorAll('[data-file]').forEach((a) => {
      a.href = `https://github.com/${REPO}/releases/download/${tag}/${a.dataset.file}`;
    });
    const version = document.getElementById('version');
    if (version) version.textContent = tag.replace(/^v/, '');
    const button = document.getElementById('primary-download');
    if (button && os) {
      button.href = `https://github.com/${REPO}/releases/download/${tag}/${PRIMARY[os][0]}`;
      document.getElementById('primary-label').textContent = PRIMARY[os][1];
    }
  };

  const ready = (fn) => (document.readyState === 'loading' ? document.addEventListener('DOMContentLoaded', fn) : fn());
  ready(() => {
    if (os) document.querySelector(`.files li[data-os="${os}"]`)?.classList.add('mine');
    apply(FALLBACK);
    fetch(`https://api.github.com/repos/${REPO}/releases?per_page=1`, { headers: { Accept: 'application/vnd.github+json' } })
      .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
      .then((list) => { if (list[0]?.tag_name) apply(list[0].tag_name); })
      .catch(() => {});
  });
})();
