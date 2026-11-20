package app.encheo;

import android.app.Activity;
import android.app.NativeActivity;
import android.graphics.PixelFormat;
import android.media.AudioAttributes;
import android.media.AudioDeviceInfo;
import android.media.AudioFormat;
import android.media.AudioManager;
import android.media.AudioTrack;
import android.os.Build;
import android.os.Bundle;
import android.view.Display;
import android.view.Surface;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.view.ViewGroup;

/**
 * The app's activity: Rust (libencheo.so) draws the UI with Slint, as in a plain NativeActivity.
 *
 * <p>On top of that it offers a video layer behind the UI. HDR video goes from the hardware
 * decoder straight onto that layer, so the TV receives real HDR instead of a version converted
 * to normal brightness. The UI's window is see-through where the player screen leaves the video
 * visible. Rust shows and hides the layer and receives its surface through {@link #surface}.
 */
public class EncheoActivity extends NativeActivity {
    private SurfaceView video;

    @Override
    protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        // NativeActivity makes the window opaque; the video layer has to show through it.
        getWindow().setFormat(PixelFormat.TRANSLUCENT);
        video = new SurfaceView(this);
        video.getHolder().addCallback(new VideoLayer());
        video.setVisibility(View.GONE);
        // A SurfaceView's picture sits behind its window: under everything Slint draws.
        addContentView(video, new ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
    }

    /** The video layer's surface appeared (non-null) or is about to go away (null). Rust, registered at startup. */
    static native void surface(Surface s);

    /**
     * The video layer's surface. (A separate object: NativeActivity's own surfaceCreated & co. are
     * the UI window's, handed to Rust.)
     */
    private static class VideoLayer implements SurfaceHolder.Callback {
        @Override
        public void surfaceCreated(SurfaceHolder holder) {
            surface(holder.getSurface());
        }

        @Override
        public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {}

        @Override
        public void surfaceDestroyed(SurfaceHolder holder) {
            // Returns only once the player has let go of it.
            surface(null);
        }
    }

    /** From Rust, any thread: show or hide the video layer. */
    public static void showVideo(Activity activity, boolean show) {
        if (!(activity instanceof EncheoActivity)) return;
        EncheoActivity a = (EncheoActivity) activity;
        a.runOnUiThread(() -> {
            if (a.video != null) a.video.setVisibility(show ? View.VISIBLE : View.GONE);
        });
    }

    /**
     * What the screen and sound output take as is, for Rust to choose how to play each video:
     * {@code hdr=hdr10,hlg;audio=ac3,eac3,atmos;hdmi=1}.
     */
    @SuppressWarnings("deprecation")
    public static String capabilities(Activity a) {
        StringBuilder hdr = new StringBuilder();
        try {
            Display d = a.getWindowManager().getDefaultDisplay();
            int[] types;
            if (Build.VERSION.SDK_INT >= 34) {
                types = d.getMode().getSupportedHdrTypes();
            } else {
                Display.HdrCapabilities c = d.getHdrCapabilities();
                types = c == null ? new int[0] : c.getSupportedHdrTypes();
            }
            for (int t : types) {
                switch (t) {
                    case Display.HdrCapabilities.HDR_TYPE_DOLBY_VISION: add(hdr, "dv"); break;
                    case Display.HdrCapabilities.HDR_TYPE_HDR10: add(hdr, "hdr10"); break;
                    case Display.HdrCapabilities.HDR_TYPE_HLG: add(hdr, "hlg"); break;
                    case Display.HdrCapabilities.HDR_TYPE_HDR10_PLUS: add(hdr, "hdr10+"); break;
                    default: break;
                }
            }
        } catch (Exception ignored) {
            // No display information: treat as SDR.
        }

        // Surround sound only goes out untouched over HDMI (a TV box, or a TV's ARC to a
        // soundbar or receiver); a phone's speaker or a TV's own speakers get decoded sound.
        StringBuilder audio = new StringBuilder();
        boolean hdmi = false;
        try {
            AudioManager am = (AudioManager) a.getSystemService(Activity.AUDIO_SERVICE);
            AudioDeviceInfo[] outs = am.getDevices(AudioManager.GET_DEVICES_OUTPUTS);
            for (AudioDeviceInfo dev : outs) {
                int t = dev.getType();
                // 29: TYPE_HDMI_EARC (Android 12).
                if (t == AudioDeviceInfo.TYPE_HDMI || t == AudioDeviceInfo.TYPE_HDMI_ARC || t == 29) hdmi = true;
            }
            int[] encodings = {AudioFormat.ENCODING_AC3, AudioFormat.ENCODING_E_AC3, AudioFormat.ENCODING_DTS, AudioFormat.ENCODING_DTS_HD, AudioFormat.ENCODING_DOLBY_TRUEHD};
            String[] names = {"ac3", "eac3", "dts", "dts-hd", "truehd"};
            if (hdmi && Build.VERSION.SDK_INT >= 29) {
                AudioAttributes attrs = new AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_MEDIA)
                        .setContentType(AudioAttributes.CONTENT_TYPE_MOVIE)
                        .build();
                for (int i = 0; i < encodings.length; i++) {
                    if (direct(encodings[i], attrs)) add(audio, names[i]);
                }
                // Dolby Atmos inside Dolby Digital Plus (streaming-style Atmos) passes with eac3.
                if (direct(AudioFormat.ENCODING_E_AC3_JOC, attrs)) add(audio, "atmos");
            } else if (hdmi) {
                for (AudioDeviceInfo dev : outs) {
                    int t = dev.getType();
                    if (t != AudioDeviceInfo.TYPE_HDMI && t != AudioDeviceInfo.TYPE_HDMI_ARC) continue;
                    for (int e : dev.getEncodings()) {
                        for (int i = 0; i < encodings.length; i++) {
                            if (e == encodings[i] && audio.indexOf(names[i]) < 0) add(audio, names[i]);
                        }
                    }
                }
            }
        } catch (Exception ignored) {
            // No audio information: decode everything.
        }
        return "hdr=" + hdr + ";audio=" + audio + ";hdmi=" + (hdmi ? "1" : "0");
    }

    /** Whether this format can go to the output untouched, as stereo or as 7.1. */
    private static boolean direct(int encoding, AudioAttributes attrs) {
        for (int mask : new int[] {AudioFormat.CHANNEL_OUT_STEREO, AudioFormat.CHANNEL_OUT_7POINT1_SURROUND}) {
            try {
                AudioFormat f = new AudioFormat.Builder().setEncoding(encoding).setSampleRate(48000).setChannelMask(mask).build();
                if (AudioTrack.isDirectPlaybackSupported(f, attrs)) return true;
            } catch (Exception ignored) {
                // Unknown encoding on this Android version.
            }
        }
        return false;
    }

    private static void add(StringBuilder sb, String s) {
        if (sb.length() > 0) sb.append(',');
        sb.append(s);
    }
}
