package com.communik8.nik8native

import android.annotation.SuppressLint
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaRecorder
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.AudioEffect
import android.media.audiofx.AutomaticGainControl
import android.media.audiofx.NoiseSuppressor
import android.os.Process
import java.util.concurrent.LinkedBlockingDeque
import java.util.concurrent.TimeUnit
import kotlin.math.sqrt

private const val SAMPLE_RATE = 16_000
/** 60 ms of PCM16 mono: one Opus frame. */
private const val CHUNK_BYTES = 960 * 2

/** RMS level of little-endian PCM16 in [0, 1]. */
private fun level(pcm: ByteArray, length: Int = pcm.size): Float {
  var sum = 0.0
  var i = 0
  while (i + 1 < length) {
    val sample = ((pcm[i + 1].toInt() shl 8) or (pcm[i].toInt() and 0xFF)).toShort() / 32768.0
    sum += sample * sample
    i += 2
  }
  val samples = length / 2
  return if (samples == 0) 0f else sqrt(sum / samples).toFloat()
}

/** Captures 16 kHz mono PCM16 in 60 ms chunks on its own thread. Requires RECORD_AUDIO. */
class VoiceRecorder(
  private val onPcm: (ByteArray) -> Unit,
  private val onLevel: (Float) -> Unit,
) {
  @Volatile private var running = false
  private var thread: Thread? = null

  @SuppressLint("MissingPermission") // checked by the module before starting
  fun start() {
    val minBuffer = AudioRecord.getMinBufferSize(SAMPLE_RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
    val record = AudioRecord(
      MediaRecorder.AudioSource.VOICE_COMMUNICATION,
      SAMPLE_RATE,
      AudioFormat.CHANNEL_IN_MONO,
      AudioFormat.ENCODING_PCM_16BIT,
      maxOf(minBuffer, CHUNK_BYTES * 4),
    )
    if (record.state != AudioRecord.STATE_INITIALIZED) {
      record.release()
      throw IllegalStateException("Microphone unavailable")
    }
    val effects = enableVoiceEffects(record.audioSessionId)

    running = true
    record.startRecording()
    thread = Thread({
      Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
      val buffer = ByteArray(CHUNK_BYTES)
      try {
        while (running) {
          val read = record.read(buffer, 0, buffer.size)
          if (read > 0) {
            onPcm(buffer.copyOf(read))
            onLevel(level(buffer, read))
          }
        }
      } finally {
        record.stop()
        record.release()
        effects.forEach { it.release() }
      }
    }, "nik8-mic").apply { start() }
  }

  /** Stops capture; the last chunk read has already been delivered. */
  fun stop() {
    running = false
    thread?.join(500)
    thread = null
  }

  private fun enableVoiceEffects(sessionId: Int): List<AudioEffect> = listOfNotNull(
    if (AcousticEchoCanceler.isAvailable()) AcousticEchoCanceler.create(sessionId) else null,
    if (NoiseSuppressor.isAvailable()) NoiseSuppressor.create(sessionId) else null,
    if (AutomaticGainControl.isAvailable()) AutomaticGainControl.create(sessionId) else null,
  ).onEach { it.enabled = true }
}

/**
 * Plays received 16 kHz mono PCM16 through the loudspeaker. Buffers a little at the
 * start of each talk burst to absorb transaction timing jitter.
 */
class VoicePlayer(private val onLevel: (Float) -> Unit) {
  private val queue = LinkedBlockingDeque<ByteArray>()
  @Volatile private var running = true

  private val track = AudioTrack.Builder()
    .setAudioAttributes(
      AudioAttributes.Builder()
        .setUsage(AudioAttributes.USAGE_MEDIA) // loudspeaker, like a walkie-talkie
        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
        .build()
    )
    .setAudioFormat(
      AudioFormat.Builder()
        .setSampleRate(SAMPLE_RATE)
        .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
        .build()
    )
    .setBufferSizeInBytes(
      maxOf(
        AudioTrack.getMinBufferSize(SAMPLE_RATE, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_16BIT),
        CHUNK_BYTES * 4,
      )
    )
    .setTransferMode(AudioTrack.MODE_STREAM)
    .build()

  private val thread = Thread({
    Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
    track.play()
    var buffering = true
    while (running) {
      if (buffering) {
        // A burst has started; give the next transaction a moment to land first.
        val first = queue.pollFirst(PREBUFFER_WAIT_MS, TimeUnit.MILLISECONDS) ?: continue
        queue.offerFirst(first)
        if (queue.size < PREBUFFER_PACKETS) Thread.sleep(PREBUFFER_WAIT_MS)
        buffering = false
      }
      val pcm = queue.pollFirst(GAP_MS, TimeUnit.MILLISECONDS)
      if (pcm == null) {
        buffering = true // burst over; buffer again before the next one
        onLevel(0f)
        continue
      }
      track.write(pcm, 0, pcm.size)
      onLevel(level(pcm))
    }
    track.stop()
    track.release()
  }, "nik8-speaker").apply { start() }

  fun enqueue(pcm: ByteArray) {
    queue.offerLast(pcm)
    // Never let latency build up: drop the oldest audio beyond ~2 s.
    while (queue.size > MAX_QUEUED_PACKETS) queue.pollFirst()
  }

  fun release() {
    running = false
    thread.join(500)
  }

  private companion object {
    /** Two 120 ms packets: one transaction's worth. */
    const val PREBUFFER_PACKETS = 2
    const val PREBUFFER_WAIT_MS = 150L
    const val GAP_MS = 400L
    const val MAX_QUEUED_PACKETS = 16
  }
}
