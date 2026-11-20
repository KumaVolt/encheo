package app.encheo;

import android.annotation.SuppressLint;
import android.app.Activity;
import android.content.Intent;
import android.graphics.Color;
import android.net.Uri;
import android.os.Bundle;
import android.view.KeyEvent;
import android.view.View;
import android.view.WindowManager;
import android.webkit.JavascriptInterface;
import android.webkit.WebChromeClient;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;

/**
 * A YouTube trailer, full screen inside Encheo, in YouTube's own embedded player (the way
 * YouTube's terms allow other apps to show its videos).
 *
 * <p>Remote: OK or Play/Pause pauses, Left/Right skip 10 seconds, Back returns to the title.
 * The trailer closes by itself when it ends. When the video's owner doesn't allow it to be shown
 * outside YouTube, it opens in the YouTube app (or browser) instead.
 */
public class TrailerActivity extends Activity {
    public static final String EXTRA_VIDEO = "video";

    private WebView web;
    private String video;

    @SuppressLint("SetJavaScriptEnabled")
    @Override
    protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        video = getIntent().getStringExtra(EXTRA_VIDEO);
        if (video == null || !video.matches("[A-Za-z0-9_-]{6,20}")) {
            finish();
            return;
        }
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        getWindow().getDecorView().setSystemUiVisibility(
                View.SYSTEM_UI_FLAG_FULLSCREEN
                        | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                        | View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                        | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                        | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);

        web = new WebView(this);
        web.setBackgroundColor(Color.BLACK);
        WebSettings s = web.getSettings();
        s.setJavaScriptEnabled(true);
        s.setDomStorageEnabled(true);
        s.setMediaPlaybackRequiresUserGesture(false);
        web.setWebChromeClient(new WebChromeClient());
        web.setWebViewClient(new WebViewClient());
        web.addJavascriptInterface(new Bridge(), "Encheo");
        setContentView(web);
        // The page's address identifies the app to YouTube (its embeds need a referrer).
        web.loadDataWithBaseURL("https://app.encheo/", page(video), "text/html", "utf-8", null);
    }

    private static String page(String id) {
        return "<!doctype html><html><head><meta name=viewport content='width=device-width,initial-scale=1'>"
                + "<style>html,body{margin:0;height:100%;background:#000;overflow:hidden}#p{width:100%;height:100%}</style>"
                + "</head><body><div id=p></div>"
                + "<script src='https://www.youtube.com/iframe_api'></script><script>"
                + "var player;"
                + "function onYouTubeIframeAPIReady(){player=new YT.Player('p',{videoId:'" + id + "',"
                + "playerVars:{autoplay:1,playsinline:1,rel:0,controls:1,fs:0,iv_load_policy:3,origin:'https://app.encheo'},"
                + "events:{onReady:function(e){e.target.playVideo();},"
                + "onStateChange:function(e){if(e.data===0)Encheo.ended();},"
                + "onError:function(e){Encheo.failed(e.data);}}});}"
                + "function toggle(){if(!player)return;if(player.getPlayerState()===1)player.pauseVideo();else player.playVideo();}"
                + "function skip(d){if(player)player.seekTo(Math.max(0,player.getCurrentTime()+d),true);}"
                + "</script></body></html>";
    }

    private class Bridge {
        @JavascriptInterface
        public void ended() {
            runOnUiThread(TrailerActivity.this::finish);
        }

        /** 101/150: the owner doesn't allow embedding; anything else: can't play here. */
        @JavascriptInterface
        public void failed(int code) {
            runOnUiThread(() -> {
                try {
                    startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse("https://www.youtube.com/watch?v=" + video)));
                } catch (Exception ignored) {
                    // No YouTube app or browser on this device.
                }
                finish();
            });
        }
    }

    @Override
    public boolean dispatchKeyEvent(KeyEvent e) {
        if (web == null || e.getAction() != KeyEvent.ACTION_DOWN) {
            return super.dispatchKeyEvent(e);
        }
        switch (e.getKeyCode()) {
            case KeyEvent.KEYCODE_DPAD_CENTER:
            case KeyEvent.KEYCODE_ENTER:
            case KeyEvent.KEYCODE_SPACE:
            case KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE:
            case KeyEvent.KEYCODE_MEDIA_PLAY:
            case KeyEvent.KEYCODE_MEDIA_PAUSE:
                web.evaluateJavascript("toggle()", null);
                return true;
            case KeyEvent.KEYCODE_DPAD_LEFT:
            case KeyEvent.KEYCODE_MEDIA_REWIND:
                web.evaluateJavascript("skip(-10)", null);
                return true;
            case KeyEvent.KEYCODE_DPAD_RIGHT:
            case KeyEvent.KEYCODE_MEDIA_FAST_FORWARD:
                web.evaluateJavascript("skip(10)", null);
                return true;
            default:
                return super.dispatchKeyEvent(e);
        }
    }

    @Override
    protected void onPause() {
        super.onPause();
        // Leaving (Home button, another app): stop the sound.
        if (web != null) web.evaluateJavascript("if(player)player.pauseVideo()", null);
    }

    @Override
    protected void onDestroy() {
        if (web != null) {
            web.destroy();
            web = null;
        }
        super.onDestroy();
    }
}
