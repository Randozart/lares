package dev.randozart.lares.hud;

import android.graphics.SurfaceTexture;
import android.hardware.Camera;

/**
 * Headless camera shim: one JPEG per call, no preview UI.
 *
 * The only Java in the Lares HUD — the API 22 camera is reachable solely
 * through the framework, and picture callbacks require a real class.
 * Called via JNI from the Rust render loop on temple tap.
 */
public final class CameraShim {
    private CameraShim() {
    }

    /** Open the camera, grab one JPEG, release. Blocking; ~2-4s. */
    public static byte[] capture() {
        Camera camera = null;
        try {
            camera = Camera.open();
            try {
                camera.enableShutterSound(false);
            } catch (Throwable ignored) {
            }
            // Headless trick: a detached SurfaceTexture satisfies the
            // preview requirement without a visible surface.
            camera.setPreviewTexture(new SurfaceTexture(0));
            camera.startPreview();

            final byte[][] result = new byte[1][];
            final Object lock = new Object();
            Camera.PictureCallback callback = new Camera.PictureCallback() {
                @Override
                public void onPictureTaken(byte[] data, Camera cam) {
                    synchronized (lock) {
                        result[0] = data;
                        lock.notifyAll();
                    }
                }
            };
            synchronized (lock) {
                camera.takePicture(null, null, callback);
                lock.wait(8000);
            }
            return result[0];
        } catch (Throwable t) {
            return null;
        } finally {
            if (camera != null) {
                try {
                    camera.stopPreview();
                } catch (Throwable ignored) {
                }
                try {
                    camera.release();
                } catch (Throwable ignored) {
                }
            }
        }
    }
}
