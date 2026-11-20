package app.encheo;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.pm.ServiceInfo;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.drawable.Icon;
import android.media.AudioAttributes;
import android.media.AudioFocusRequest;
import android.media.AudioManager;
import android.media.MediaMetadata;
import android.media.session.MediaSession;
import android.media.session.PlaybackState;
import android.net.wifi.WifiManager;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.os.PowerManager;
import android.os.SystemClock;
import java.io.InputStream;
import java.net.URL;

/**
 * Music in the background, like any music app:
 *
 * <ul>
 *   <li>a media session, so the song and its controls show on the lock screen and in the
 *       notification shade, and headphone and Bluetooth buttons work;
 *   <li>a foreground service with a media notification, so Android keeps Encheo running while
 *       music plays with the screen off or another app open;
 *   <li>audio focus: pause when another app starts playing or a call comes in, and when
 *       headphones are unplugged.
 * </ul>
 *
 * <p>The Rust side (src/android_media.rs) calls {@link #update} and {@link #stop}; buttons go
 * back to it through {@link #command}, which Rust registers.
 */
public class MusicService extends Service {
    /** "play", "pause", "next", "prev", "stop" or "seek:<ms>"; handled in Rust. */
    static native void command(String cmd);

    private static final String CHANNEL = "music";
    private static final int NOTIFICATION = 1;
    private static final Handler main = new Handler(Looper.getMainLooper());

    // What's playing. Only touched on the main thread.
    private static Context app;
    private static MediaSession session;
    private static MusicService running;
    private static boolean starting;
    private static String title = "", artist = "", album = "", artUrl = "";
    private static Bitmap art;
    private static long duration, position;
    private static boolean playing;
    private static AudioFocusRequest focus;
    private static boolean resumeOnFocus;

    private PowerManager.WakeLock wakeLock;
    private WifiManager.WifiLock wifiLock;
    private final BroadcastReceiver noisy = new BroadcastReceiver() {
        @Override
        public void onReceive(Context context, Intent intent) {
            send("pause");
        }
    };

    /** The song, its position and whether it plays. Called by Rust when any of them changes. */
    public static void update(Context context, String songTitle, String songArtist, String songAlbum,
            String coverUrl, long durationMs, long positionMs, boolean isPlaying) {
        Context appContext = context.getApplicationContext();
        main.post(() -> {
            app = appContext;
            boolean wasPlaying = playing;
            title = songTitle;
            artist = songArtist;
            album = songAlbum;
            duration = durationMs;
            position = positionMs;
            playing = isPlaying;
            if (!coverUrl.equals(artUrl)) {
                artUrl = coverUrl;
                art = null;
                fetchArt(coverUrl);
            }
            ensureSession();
            if (playing && !wasPlaying) {
                requestFocus();
            }
            if (running == null && !starting) {
                starting = true;
                try {
                    app.startForegroundService(new Intent(app, MusicService.class));
                } catch (RuntimeException e) {
                    // Not allowed from the background on this Android version; the session
                    // still shows the controls.
                    starting = false;
                }
            }
            publish();
        });
    }

    /** Music closed: remove the controls and let Android stop the service. */
    public static void stop() {
        main.post(() -> {
            playing = false;
            abandonFocus();
            if (session != null) {
                session.setActive(false);
                session.release();
                session = null;
            }
            if (running != null) {
                running.stopForeground(STOP_FOREGROUND_REMOVE);
                running.stopSelf();
            }
            title = artist = album = artUrl = "";
            art = null;
        });
    }

    private static void send(String cmd) {
        try {
            command(cmd);
        } catch (UnsatisfiedLinkError e) {
            // The Rust side isn't loaded (the app was closed); nothing to control.
        }
    }

    private static void ensureSession() {
        if (session != null) {
            return;
        }
        session = new MediaSession(app, "Encheo");
        session.setCallback(new MediaSession.Callback() {
            @Override public void onPlay() { send("play"); }
            @Override public void onPause() { send("pause"); }
            @Override public void onSkipToNext() { send("next"); }
            @Override public void onSkipToPrevious() { send("prev"); }
            @Override public void onStop() { send("stop"); }
            @Override public void onSeekTo(long pos) { send("seek:" + pos); }
        });
        // Tapping the controls opens Encheo.
        Intent open = app.getPackageManager().getLaunchIntentForPackage(app.getPackageName());
        if (open != null) {
            open.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_RESET_TASK_IF_NEEDED);
            session.setSessionActivity(PendingIntent.getActivity(app, 0, open, PendingIntent.FLAG_IMMUTABLE));
        }
        session.setActive(true);
    }

    /** Pushes the current song to the session and the notification. */
    private static void publish() {
        if (session == null) {
            return;
        }
        MediaMetadata.Builder meta = new MediaMetadata.Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, title)
                .putString(MediaMetadata.METADATA_KEY_ARTIST, artist)
                .putString(MediaMetadata.METADATA_KEY_ALBUM, album)
                .putLong(MediaMetadata.METADATA_KEY_DURATION, duration);
        if (art != null) {
            meta.putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, art);
        }
        session.setMetadata(meta.build());
        session.setPlaybackState(new PlaybackState.Builder()
                .setActions(PlaybackState.ACTION_PLAY | PlaybackState.ACTION_PAUSE
                        | PlaybackState.ACTION_PLAY_PAUSE | PlaybackState.ACTION_SKIP_TO_NEXT
                        | PlaybackState.ACTION_SKIP_TO_PREVIOUS | PlaybackState.ACTION_SEEK_TO
                        | PlaybackState.ACTION_STOP)
                .setState(playing ? PlaybackState.STATE_PLAYING : PlaybackState.STATE_PAUSED,
                        position, playing ? 1f : 0f, SystemClock.elapsedRealtime())
                .build());
        if (running != null) {
            running.getSystemService(NotificationManager.class).notify(NOTIFICATION, running.notification());
            running.holdLocks(playing);
        }
    }

    private static void fetchArt(String url) {
        if (url.isEmpty()) {
            return;
        }
        new Thread(() -> {
            Bitmap bitmap = null;
            try (InputStream in = new URL(url).openStream()) {
                bitmap = BitmapFactory.decodeStream(in);
            } catch (Exception e) {
                // No cover; the controls show without one.
            }
            Bitmap result = bitmap;
            main.post(() -> {
                if (url.equals(artUrl) && result != null) {
                    art = result;
                    publish();
                }
            });
        }, "encheo-cover").start();
    }

    // ---------- Audio focus ----------

    private static void requestFocus() {
        AudioManager audio = app.getSystemService(AudioManager.class);
        if (focus == null) {
            focus = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                    .setAudioAttributes(new AudioAttributes.Builder()
                            .setUsage(AudioAttributes.USAGE_MEDIA)
                            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                            .build())
                    .setOnAudioFocusChangeListener(change -> {
                        switch (change) {
                            case AudioManager.AUDIOFOCUS_LOSS:
                                // Another app plays music: pause and stay paused.
                                resumeOnFocus = false;
                                send("pause");
                                break;
                            case AudioManager.AUDIOFOCUS_LOSS_TRANSIENT:
                                // A call or a voice prompt: pause, and carry on afterwards.
                                resumeOnFocus = playing;
                                send("pause");
                                break;
                            case AudioManager.AUDIOFOCUS_GAIN:
                                if (resumeOnFocus) {
                                    resumeOnFocus = false;
                                    send("play");
                                }
                                break;
                            default:
                                // Ducking (a navigation prompt): Android lowers the volume itself.
                                break;
                        }
                    }, main)
                    .build();
        }
        audio.requestAudioFocus(focus);
    }

    private static void abandonFocus() {
        if (focus != null && app != null) {
            app.getSystemService(AudioManager.class).abandonAudioFocusRequest(focus);
        }
        resumeOnFocus = false;
    }

    // ---------- The service ----------

    @Override
    public void onCreate() {
        super.onCreate();
        NotificationChannel channel = new NotificationChannel(CHANNEL, "Music", NotificationManager.IMPORTANCE_LOW);
        channel.setShowBadge(false);
        getSystemService(NotificationManager.class).createNotificationChannel(channel);
        IntentFilter filter = new IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY);
        if (Build.VERSION.SDK_INT >= 33) {
            registerReceiver(noisy, filter, Context.RECEIVER_NOT_EXPORTED);
        } else {
            registerReceiver(noisy, filter);
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        running = this;
        starting = false;
        if (session == null) {
            // Music was closed before the service got going.
            stopSelf();
            return START_NOT_STICKY;
        }
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(NOTIFICATION, notification(), ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK);
        } else {
            startForeground(NOTIFICATION, notification());
        }
        holdLocks(playing);
        // The notification's buttons arrive here.
        String action = intent != null ? intent.getAction() : null;
        if (action != null) {
            send(action);
        }
        return START_NOT_STICKY;
    }

    @Override
    public void onDestroy() {
        unregisterReceiver(noisy);
        holdLocks(false);
        if (running == this) {
            running = null;
        }
        starting = false;
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    private Notification notification() {
        Notification.Builder b = new Notification.Builder(this, CHANNEL)
                .setSmallIcon(R.drawable.ic_stat_encheo)
                .setContentTitle(title)
                .setContentText(artist)
                .setSubText(album)
                .setLargeIcon(art)
                .setVisibility(Notification.VISIBILITY_PUBLIC)
                .setShowWhen(false)
                .setOngoing(playing)
                .addAction(action(android.R.drawable.ic_media_previous, "Previous", "prev"))
                .addAction(playing
                        ? action(android.R.drawable.ic_media_pause, "Pause", "pause")
                        : action(android.R.drawable.ic_media_play, "Play", "play"))
                .addAction(action(android.R.drawable.ic_media_next, "Next", "next"))
                .setStyle(new Notification.MediaStyle()
                        .setMediaSession(session != null ? session.getSessionToken() : null)
                        .setShowActionsInCompactView(0, 1, 2));
        if (session != null) {
            b.setContentIntent(session.getController().getSessionActivity());
        }
        return b.build();
    }

    private Notification.Action action(int icon, String label, String cmd) {
        Intent intent = new Intent(this, MusicService.class).setAction(cmd);
        PendingIntent pending = PendingIntent.getService(this, cmd.hashCode(), intent,
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        return new Notification.Action.Builder(Icon.createWithResource(this, icon), label, pending).build();
    }

    /** Keep the CPU and Wi-Fi awake while music plays (streams stall otherwise); not when paused. */
    private void holdLocks(boolean hold) {
        if (wakeLock == null) {
            wakeLock = getSystemService(PowerManager.class).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "encheo:music");
            wakeLock.setReferenceCounted(false);
            wifiLock = getSystemService(WifiManager.class).createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "encheo:music");
            wifiLock.setReferenceCounted(false);
        }
        if (hold) {
            wakeLock.acquire();
            wifiLock.acquire();
        } else {
            if (wakeLock.isHeld()) wakeLock.release();
            if (wifiLock.isHeld()) wifiLock.release();
        }
    }
}
