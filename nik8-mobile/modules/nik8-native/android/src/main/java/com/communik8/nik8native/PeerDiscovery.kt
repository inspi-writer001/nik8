package com.communik8.nik8native

import android.annotation.SuppressLint
import android.bluetooth.BluetoothManager
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.bluetooth.le.BluetoothLeAdvertiser
import android.bluetooth.le.BluetoothLeScanner
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanRecord
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.ParcelUuid
import java.util.UUID
import kotlin.math.pow

/**
 * Finds nearby communik8 devices over BLE without connecting to them.
 *
 * Each device advertises its 32-byte session address split across one
 * non-connectable advertisement and its scan response, so an active scanner gets
 * the whole address from a single scan result:
 *   advertisement: manufacturer data (company 0xFFFF) = [version][address[0 until k]]
 *   scan response: service data (UUID 0xFFF8)        = [address[k until 32]]
 * k is 26 normally; 23 if the stack adds the 3-byte flags field.
 */
class PeerDiscovery(
  context: Context,
  private val onPeers: (List<Peer>) -> Unit,
  private val onError: (String) -> Unit,
) {
  data class Peer(val address: ByteArray, val rssi: Int, val distanceMeters: Double?)

  private class Sighting(val address: ByteArray, var rssi: Double, var txPower: Int?, var lastSeen: Long)

  private val adapter = context.getSystemService(BluetoothManager::class.java)?.adapter
  private val handler = Handler(Looper.getMainLooper())
  private val sightings = HashMap<String, Sighting>()
  private var advertiser: BluetoothLeAdvertiser? = null
  private var scanner: BluetoothLeScanner? = null
  private var running = false
  private var address: ByteArray? = null
  private var advertisedFirstLen = FIRST_PART_LEN

  @SuppressLint("MissingPermission") // the module checks BLUETOOTH_SCAN/ADVERTISE first
  fun start(address: ByteArray) {
    require(address.size == ADDRESS_LEN) { "address must be 32 bytes" }
    val adapter = adapter ?: return onError("Bluetooth unavailable")
    if (!adapter.isEnabled) return onError("Bluetooth is off")
    stop()
    running = true
    this.address = address

    advertiser = adapter.bluetoothLeAdvertiser?.also { advertise(it, address, FIRST_PART_LEN) }
      ?: return onError("This device can't advertise over BLE")

    scanner = adapter.bluetoothLeScanner?.also {
      val filter = ScanFilter.Builder()
        .setManufacturerData(COMPANY_ID, byteArrayOf(VERSION), byteArrayOf(0xFF.toByte()))
        .build()
      val settings = ScanSettings.Builder()
        .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY) // active scan: also fetches scan responses
        .setCallbackType(ScanSettings.CALLBACK_TYPE_ALL_MATCHES)
        .build()
      it.startScan(listOf(filter), settings, scanCallback)
    } ?: return onError("This device can't scan over BLE")

    handler.post(publish)
  }

  @SuppressLint("MissingPermission")
  fun stop() {
    if (!running) return
    running = false
    handler.removeCallbacks(publish)
    runCatching { advertiser?.stopAdvertising(advertiseCallback) }
    runCatching { scanner?.stopScan(scanCallback) }
    advertiser = null
    scanner = null
    address = null
    sightings.clear()
  }

  @SuppressLint("MissingPermission")
  private fun advertise(advertiser: BluetoothLeAdvertiser, address: ByteArray, firstLen: Int) {
    val settings = AdvertiseSettings.Builder()
      .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_LATENCY)
      .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_MEDIUM)
      .setConnectable(false)
      .build()
    val data = AdvertiseData.Builder()
      .addManufacturerData(COMPANY_ID, byteArrayOf(VERSION) + address.copyOfRange(0, firstLen))
      .build()
    val scanResponse = AdvertiseData.Builder()
      .addServiceData(SERVICE_UUID, address.copyOfRange(firstLen, ADDRESS_LEN))
      .setIncludeTxPowerLevel(true) // lets scanners estimate distance
      .build()
    advertisedFirstLen = firstLen
    advertiser.startAdvertising(settings, data, scanResponse, advertiseCallback)
  }

  private val advertiseCallback = object : AdvertiseCallback() {
    override fun onStartFailure(errorCode: Int) {
      val advertiser = advertiser
      val address = address
      // Some stacks add the flags field even when non-connectable; move 3 bytes over.
      if (errorCode == ADVERTISE_FAILED_DATA_TOO_LARGE && advertisedFirstLen == FIRST_PART_LEN &&
        advertiser != null && address != null
      ) {
        advertise(advertiser, address, FIRST_PART_LEN_WITH_FLAGS)
        return
      }
      onError("BLE advertising failed ($errorCode)")
    }
  }

  private val scanCallback = object : ScanCallback() {
    override fun onScanResult(callbackType: Int, result: ScanResult) {
      val record = result.scanRecord ?: return
      val address = decodeAddress(record) ?: return
      val key = address.toHex()
      val now = System.currentTimeMillis()
      val txPower = record.txPowerLevel.takeIf { it != Int.MIN_VALUE }
      val sighting = sightings[key]
      if (sighting == null) {
        sightings[key] = Sighting(address, result.rssi.toDouble(), txPower, now)
      } else {
        // Smooth RSSI; single readings jump by 10 dB or more.
        sighting.rssi = sighting.rssi * (1 - RSSI_SMOOTHING) + result.rssi * RSSI_SMOOTHING
        sighting.txPower = txPower ?: sighting.txPower
        sighting.lastSeen = now
      }
    }

    override fun onScanFailed(errorCode: Int) {
      onError("BLE scan failed ($errorCode)")
    }
  }

  private val publish = object : Runnable {
    override fun run() {
      if (!running) return
      val now = System.currentTimeMillis()
      sightings.values.removeAll { now - it.lastSeen > STALE_MS }
      onPeers(
        sightings.values
          .map { Peer(it.address, it.rssi.toInt(), estimateDistance(it.rssi, it.txPower)) }
          .sortedByDescending { it.rssi }
      )
      handler.postDelayed(this, PUBLISH_MS)
    }
  }

  private fun decodeAddress(record: ScanRecord): ByteArray? {
    val first = record.getManufacturerSpecificData(COMPANY_ID) ?: return null
    if (first.isEmpty() || first[0] != VERSION) return null
    // Without the scan response (passive scan) the address is incomplete; ignore it.
    val rest = record.getServiceData(SERVICE_UUID) ?: return null
    val address = first.copyOfRange(1, first.size) + rest
    return address.takeIf { it.size == ADDRESS_LEN }
  }

  /**
   * Log-distance path loss. Tx power is measured at 0 m; ~41 dB is typical loss at 1 m.
   * Rough: bodies, pockets and cars easily double or halve the estimate.
   */
  private fun estimateDistance(rssi: Double, txPower: Int?): Double? {
    txPower ?: return null
    val measuredAtOneMeter = txPower - 41
    return 10.0.pow((measuredAtOneMeter - rssi) / (10 * PATH_LOSS_EXPONENT))
  }

  private companion object {
    const val ADDRESS_LEN = 32
    const val VERSION: Byte = 1
    /** Bluetooth SIG "reserved for testing" company ID. Use an assigned ID for release. */
    const val COMPANY_ID = 0xFFFF
    val SERVICE_UUID: ParcelUuid = ParcelUuid(UUID.fromString("0000fff8-0000-1000-8000-00805f9b34fb"))
    /** 31 bytes - 4 (length, type, company ID) - 1 (version). */
    const val FIRST_PART_LEN = 26
    const val FIRST_PART_LEN_WITH_FLAGS = 23
    const val RSSI_SMOOTHING = 0.3
    const val PATH_LOSS_EXPONENT = 2.0
    const val STALE_MS = 5_000L
    const val PUBLISH_MS = 500L
  }
}

private fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it) }
