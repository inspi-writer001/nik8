package com.communik8.nik8native

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.util.Log
import androidx.core.content.ContextCompat
import com.communik8.nik8ffi.ListenerCallback
import com.communik8.nik8ffi.ListenerState
import com.communik8.nik8ffi.ListenerStats
import com.communik8.nik8ffi.Nik8Client
import com.communik8.nik8ffi.Nik8FfiException
import com.communik8.nik8ffi.TalkCallback
import com.communik8.nik8ffi.parseVoicePacket
import expo.modules.kotlin.exception.CodedException
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import expo.modules.kotlin.typedarray.Uint8Array
import kotlinx.coroutines.launch
import com.communik8.nik8ffi.version as nik8FfiVersion

class VoicePacketParseException(message: String?) :
  CodedException("ERR_VOICE_PACKET", message, null)

class ListenerStartException(message: String?) :
  CodedException("ERR_LISTENER_START", message, null)

private const val TAG = "Nik8"

class SessionException(message: String?) :
  CodedException("ERR_SESSION", message, null)

class TalkException(message: String?) :
  CodedException("ERR_TALK", message, null)

class DiscoveryException(message: String?) :
  CodedException("ERR_DISCOVERY", message, null)

private fun discoveryPermissions(): List<String> =
  if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
    listOf(Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_ADVERTISE)
  } else {
    listOf(Manifest.permission.ACCESS_FINE_LOCATION)
  }

private inline fun <T> session(block: () -> T): T =
  try {
    block()
  } catch (e: Nik8FfiException) {
    throw SessionException(e.message)
  }

class Nik8NativeModule : Module() {
  private var client: Nik8Client? = null
  private var player: VoicePlayer? = null
  private var recorder: VoiceRecorder? = null
  private var discovery: PeerDiscovery? = null

  private fun client(): Nik8Client = client ?: Nik8Client().also { client = it }

  private fun player(): VoicePlayer =
    player ?: VoicePlayer { level -> sendLevel("speaker", level) }.also { player = it }

  private fun sendLevel(source: String, level: Float) {
    sendEvent("onAudioLevel", Bundle().apply {
      putString("source", source)
      putDouble("level", level.toDouble())
    })
  }

  private fun stopRecorder() {
    recorder?.stop()
    recorder = null
  }

  override fun definition() = ModuleDefinition {
    Name("Nik8Native")

    Events(
      "onListenerState",
      "onListenerStats",
      "onListenerFailed",
      "onTalkSent",
      "onTalkFailed",
      "onAudioLevel",
      "onPeers",
      "onDiscoveryError",
    )

    Function("version") {
      nik8FfiVersion()
    }

    Function("parseVoicePacket") { data: Uint8Array ->
      val bytes = ByteArray(data.byteLength)
      data.read(bytes, 0, data.byteLength)

      val info = try {
        parseVoicePacket(bytes)
      } catch (e: Nik8FfiException) {
        throw VoicePacketParseException(e.message)
      }

      mapOf(
        "version" to info.version.toInt(),
        "codec" to info.codec.toInt(),
        "sessionId" to info.sessionId.toHex(),
        "recipient" to info.recipient.toHex(),
        "sequence" to info.sequence.toLong(),
        "captureTimestampMs" to info.captureTimestampMs.toDouble(),
        "opusLen" to info.opusLen.toLong(),
      )
    }

    Function("startListening") { grpcEndpoint: String, grpcToken: String?, recipient: String ->
      player()
      try {
        client().startListening(grpcEndpoint, grpcToken, recipient, EventForwarder())
      } catch (e: Nik8FfiException) {
        throw ListenerStartException(e.message)
      }
    }

    Function("stopListening") {
      client?.stopListening()
      player?.release()
      player = null
    }

    // Push-to-talk. Capture and encoding run on native threads; JS only gets events.

    Function("prepareToTalk") {
      client?.prepareToTalk()
    }

    AsyncFunction("startTalking") { recipient: String ->
      val context = appContext.reactContext ?: throw TalkException("React context unavailable")
      if (ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
        throw TalkException("Microphone permission not granted")
      }
      val client = client()
      try {
        client.startTalking(recipient, TalkForwarder())
      } catch (e: Nik8FfiException) {
        throw TalkException(e.message)
      }
      try {
        recorder = VoiceRecorder(
          onPcm = { pcm -> client.pushPcm(pcm) },
          onLevel = { level -> sendLevel("mic", level) },
        ).also { it.start() }
      } catch (e: Exception) {
        client.stopTalking()
        throw TalkException(e.message)
      }
    }

    AsyncFunction("stopTalking") {
      // Stop capture first so the final chunk is queued before the flush.
      stopRecorder()
      client?.stopTalking()
    }

    // Session key. Network calls block, so these run off the JS thread as AsyncFunctions.

    AsyncFunction("initSession") { rpcUrl: String, grpcEndpoint: String, grpcToken: String?,
                                   ingressUrl: String?, ingressCertSha256: String? ->
      val context = appContext.reactContext ?: throw SessionException("React context unavailable")
      val seed = SessionKeyStore(context).loadOrCreateSeed()
      try {
        session {
          client().configureSession(seed, rpcUrl, grpcEndpoint, grpcToken, ingressUrl, ingressCertSha256)
        }
      } finally {
        seed.fill(0)
      }
    }

    Function("submitPath") {
      client?.let { runCatching { it.submitPath() }.getOrNull() }
    }

    AsyncFunction("sessionStatus") {
      val info = session { client().sessionStatus() }
      mapOf(
        "pubkey" to info.pubkey,
        "senderVault" to info.senderVault,
        "balanceLamports" to info.balanceLamports.toDouble(),
        "registered" to info.registered,
      )
    }

    AsyncFunction("registerSender") {
      session { client().registerSender() }
    }

    AsyncFunction("sendTestPacket") { recipient: String ->
      session { client().sendTestPacket(recipient) }
    }

    // Peer discovery over BLE. Runs on the main looper; results arrive every 500 ms.

    Function("startDiscovery") { address: Uint8Array ->
      val context = appContext.reactContext ?: throw DiscoveryException("React context unavailable")
      val missing = discoveryPermissions().filter {
        ContextCompat.checkSelfPermission(context, it) != PackageManager.PERMISSION_GRANTED
      }
      if (missing.isNotEmpty()) throw DiscoveryException("Bluetooth permission not granted")
      val bytes = ByteArray(address.byteLength).also { address.read(it, 0, address.byteLength) }

      val discovery = discovery ?: PeerDiscovery(
        context,
        onPeers = { peers ->
          sendEvent("onPeers", Bundle().apply {
            putParcelableArrayList("peers", ArrayList(peers.map { peer ->
              Bundle().apply {
                putString("address", peer.address.toHex())
                putInt("rssi", peer.rssi)
                peer.distanceMeters?.let { putDouble("distanceMeters", it) }
              }
            }))
          })
        },
        onError = { reason -> sendEvent("onDiscoveryError", Bundle().apply { putString("reason", reason) }) },
      ).also { discovery = it }
      appContext.mainQueue.launch { discovery.start(bytes) }
      Unit
    }

    Function("stopDiscovery") {
      discovery?.let { appContext.mainQueue.launch { it.stop() } }
      Unit
    }

    OnDestroy {
      discovery?.stop()
      discovery = null
      stopRecorder()
      client?.stopTalking()
      client?.stopListening()
      player?.release()
      player = null
      client?.close()
      client = null
    }
  }

  /** Called from Rust worker threads; Expo's sendEvent is thread-safe. */
  private inner class EventForwarder : ListenerCallback {
    override fun onState(state: ListenerState) {
      Log.i(TAG, "listener: $state")
      sendEvent("onListenerState", state.toBundle())
    }

    override fun onStats(stats: ListenerStats) {
      sendEvent("onListenerStats", stats.toBundle())
    }

    override fun onFailed(reason: String) {
      Log.w(TAG, "listener failed: $reason")
      sendEvent("onListenerFailed", Bundle().apply { putString("reason", reason) })
    }

    override fun onAudio(pcm: ByteArray) {
      player?.enqueue(pcm)
    }
  }

  private inner class TalkForwarder : TalkCallback {
    override fun onSent(packets: UInt, signature: String) {
      sendEvent("onTalkSent", Bundle().apply {
        putInt("packets", packets.toInt())
        putString("signature", signature)
      })
    }

    override fun onFailed(reason: String) {
      Log.w(TAG, "talk failed: $reason")
      sendEvent("onTalkFailed", Bundle().apply { putString("reason", reason) })
    }
  }
}

private fun ListenerState.toBundle() = Bundle().apply {
  when (val state = this@toBundle) {
    ListenerState.Connecting -> putString("state", "connecting")
    ListenerState.Online -> putString("state", "online")
    is ListenerState.Reconnecting -> {
      putString("state", "reconnecting")
      putInt("attempt", state.attempt.toInt())
      putDouble("delayMs", state.delayMs.toDouble())
      putString("reason", state.reason)
    }
  }
}

private fun ListenerStats.toBundle() = Bundle().apply {
  putDouble("accepted", accepted.toDouble())
  putDouble("duplicates", duplicates.toDouble())
  putDouble("otherRecipients", otherRecipients.toDouble())
  putDouble("rejected", rejected.toDouble())
  putDouble("unsupported", unsupported.toDouble())
  lastSequence?.let { putDouble("lastSequence", it.toDouble()) }
}

private fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it) }
