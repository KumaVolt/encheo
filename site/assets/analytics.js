// Website analytics: Google Tag Manager, only after the visitor allows statistics in the
// OpenCookie banner (loaded first in <head>, it also sets Google Consent Mode).
// Leave GTM_ID empty to turn analytics off: then nothing loads and the banner stays hidden.
(() => {
  const GTM_ID = 'GTM-N2TMJSQS';

  window.dataLayer = window.dataLayer || [];
  // Custom events, e.g. track('file_download', { file: 'encheo-macos-arm64.zip' }).
  // Sent only with consent, so nothing queues up for visitors who said no.
  window.track = (event, params) => {
    if (GTM_ID && window.OpenCookie && window.OpenCookie.hasConsent('statistics')) {
      window.dataLayer.push(Object.assign({ event }, params || {}));
    }
  };
  if (!GTM_ID) return;

  // The GTM loader, held back as text/plain until OpenCookie allows statistics.
  const s = document.createElement('script');
  s.setAttribute('type', 'text/plain');
  s.setAttribute('data-oc-category', 'statistics');
  s.text = "(function(w,d,s,l,i){w[l]=w[l]||[];w[l].push({'gtm.start':new Date().getTime(),event:'gtm.js'});" +
    "var f=d.getElementsByTagName(s)[0],j=d.createElement(s),dl=l!='dataLayer'?'&l='+l:'';j.async=true;" +
    "j.src='https://www.googletagmanager.com/gtm.js?id='+i+dl;f.parentNode.insertBefore(j,f);})(window,document,'script','dataLayer','" + GTM_ID + "');";
  document.head.appendChild(s);

  // Downloads, store and source-code clicks.
  document.addEventListener('click', (e) => {
    const a = e.target instanceof Element ? e.target.closest('a') : null;
    if (!a) return;
    if (a.dataset.file || a.id === 'primary-download') {
      window.track('file_download', { file: a.dataset.file || a.href.split('/').pop(), link_url: a.href, placement: a.id || 'download_list' });
    } else if (a.hostname === 'play.google.com') {
      window.track('store_click', { store: 'google_play', link_url: a.href });
    } else if (a.hostname === 'github.com') {
      window.track('github_click', { link_url: a.href });
    }
  });
})();
