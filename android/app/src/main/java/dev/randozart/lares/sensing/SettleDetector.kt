package dev.randozart.lares.sensing

import android.content.Context
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.os.SystemClock
import kotlin.math.sqrt

/** Number of accelerometer samples that make up the stability window. */
private const val WINDOW_SAMPLES = 30
/** Accelerometer magnitude standard deviation that counts as "still". */
private const val STILL_STDDEV = 0.25f

/**
 * Detects when the phone has stopped moving (pan-settle) using the
 * accelerometer. Fires [onSettled] once on the moving-to-still transition.
 */
class SettleDetector(
    context: Context,
    private val onSettled: () -> Unit,
) : SensorEventListener {
    private val sensorManager =
        context.getSystemService(Context.SENSOR_SERVICE) as SensorManager
    private val accelerometer = sensorManager.getDefaultSensor(Sensor.TYPE_ACCELEROMETER)

    private val samples = ArrayDeque<Float>()
    private var wasStill = false

    /** Begin listening for pan-settle. */
    fun start() {
        accelerometer?.let { sensorManager.registerListener(this, it, SensorManager.SENSOR_DELAY_NORMAL) }
    }

    /** Stop listening. */
    fun stop() {
        sensorManager.unregisterListener(this)
    }

    override fun onSensorChanged(event: SensorEvent) {
        val magnitude = sqrt(
            event.values[0] * event.values[0] +
                event.values[1] * event.values[1] +
                event.values[2] * event.values[2],
        )
        samples.addLast(magnitude)
        while (samples.size > WINDOW_SAMPLES) {
            samples.removeFirst()
        }
        if (samples.size < WINDOW_SAMPLES) return

        val mean = samples.average().toFloat()
        var variance = 0f
        for (sample in samples) {
            val delta = sample - mean
            variance += delta * delta
        }
        variance /= samples.size
        val stddev = sqrt(variance)
        val still = stddev < STILL_STDDEV
        if (still && !wasStill) {
            wasStill = true
            onSettled()
        } else if (!still) {
            wasStill = false
        }
    }

    override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) = Unit
}