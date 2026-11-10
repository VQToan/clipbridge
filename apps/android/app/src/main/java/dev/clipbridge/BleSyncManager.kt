package dev.clipbridge

import android.annotation.SuppressLint
import android.bluetooth.*
import android.bluetooth.le.*
import android.content.Context
import android.os.Build
import android.os.ParcelUuid
import android.util.Log
import dev.clipbridge.SyncEngine
import uniffi.clip_ffi.*
import java.util.*

@SuppressLint("MissingPermission")
class BleSyncManager(private val context: Context, private val onEnvelopeReceived: (FfiEnvelope) -> Unit) {

    private val bluetoothManager = context.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager
    private val adapter: BluetoothAdapter? = bluetoothManager?.adapter

    private val serviceUuid = UUID.fromString(ffiBleServiceUuid())
    private val charUuid = UUID.fromString(ffiBleCharUuid())

    private var beaconToken: ByteArray? = null
    private val reassembler = FfiBleReassembler()

    private var gattServer: BluetoothGattServer? = null
    private var advertiser: BluetoothLeAdvertiser? = null
    private var scanner: BluetoothLeScanner? = null

    private val connectedDevices = Collections.synchronizedSet(mutableSetOf<BluetoothDevice>())
    private val pendingChunks = Collections.synchronizedList(mutableListOf<ByteArray>())

    init {
        setupGattServer()
    }

    fun updateGroupKey(key: FfiGroupKey) {
        val token = ffiBleBeaconToken(key)
        this.beaconToken = token
        restartAdvertising()
        restartScanning()
    }

    fun broadcastEnvelope(envelope: FfiEnvelope) {
        try {
            val chunks = ffiBleChunkEnvelope(envelope, 490u)
            pendingChunks.clear()
            pendingChunks.addAll(chunks)

            // Send to connected central devices
            gattServer?.let { server ->
                val service = server.getService(serviceUuid)
                val char = service?.getCharacteristic(charUuid)
                if (char != null) {
                    for (device in connectedDevices) {
                        for (chunk in chunks) {
                            char.value = chunk
                            server.notifyCharacteristicChanged(device, char, false)
                        }
                    }
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to broadcast envelope via BLE", e)
        }
    }

    private fun setupGattServer() {
        if (adapter?.isEnabled != true) return
        val serverCallback = object : BluetoothGattServerCallback() {
            override fun onConnectionStateChange(device: BluetoothDevice, status: Int, newState: Int) {
                if (newState == BluetoothProfile.STATE_CONNECTED) {
                    connectedDevices.add(device)
                    Log.d(TAG, "BLE device connected: ${device.address}")
                } else if (newState == BluetoothProfile.STATE_DISCONNECTED) {
                    connectedDevices.remove(device)
                    Log.d(TAG, "BLE device disconnected: ${device.address}")
                }
            }

            override fun onCharacteristicWriteRequest(
                device: BluetoothDevice,
                requestId: Int,
                characteristic: BluetoothGattCharacteristic,
                preparedWrite: Boolean,
                responseNeeded: Boolean,
                offset: Int,
                value: ByteArray?
            ) {
                value?.let { handleIncomingChunk(it) }
                if (responseNeeded) {
                    gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, value)
                }
            }
        }

        gattServer = bluetoothManager?.openGattServer(context, serverCallback)
        val char = BluetoothGattCharacteristic(
            charUuid,
            BluetoothGattCharacteristic.PROPERTY_READ or
                    BluetoothGattCharacteristic.PROPERTY_WRITE or
                    BluetoothGattCharacteristic.PROPERTY_WRITE_NO_RESPONSE or
                    BluetoothGattCharacteristic.PROPERTY_NOTIFY,
            BluetoothGattCharacteristic.PERMISSION_READ or BluetoothGattCharacteristic.PERMISSION_WRITE
        )

        val service = BluetoothGattService(serviceUuid, BluetoothGattService.SERVICE_TYPE_PRIMARY)
        service.addCharacteristic(char)
        gattServer?.addService(service)
    }

    private fun restartAdvertising() {
        val token = beaconToken ?: return
        if (adapter?.isEnabled != true) return

        advertiser = adapter.bluetoothLeAdvertiser
        if (advertiser == null) return

        val settings = AdvertiseSettings.Builder()
            .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_BALANCED)
            .setConnectable(true)
            .setTimeout(0)
            .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_MEDIUM)
            .build()

        val data = AdvertiseData.Builder()
            .setIncludeDeviceName(false)
            .addServiceUuid(ParcelUuid(serviceUuid))
            .addServiceData(ParcelUuid(serviceUuid), token)
            .build()

        advertiser?.stopAdvertising(advertiseCallback)
        advertiser?.startAdvertising(settings, data, advertiseCallback)
    }

    private val advertiseCallback = object : AdvertiseCallback() {
        override fun onStartSuccess(settingsInEffect: AdvertiseSettings?) {
            Log.d(TAG, "BLE Advertising started successfully")
        }

        override fun onStartFailure(errorCode: Int) {
            Log.w(TAG, "BLE Advertising failed: $errorCode")
        }
    }

    private fun restartScanning() {
        if (adapter?.isEnabled != true) return
        scanner = adapter.bluetoothLeScanner
        if (scanner == null) return

        val filter = ScanFilter.Builder()
            .setServiceUuid(ParcelUuid(serviceUuid))
            .build()

        val settings = ScanSettings.Builder()
            .setScanMode(ScanSettings.SCAN_MODE_BALANCED)
            .build()

        scanner?.stopScan(scanCallback)
        scanner?.startScan(listOf(filter), settings, scanCallback)
    }

    private val scanCallback = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, result: ScanResult?) {
            val record = result?.scanRecord ?: return
            val token = beaconToken ?: return

            // Verify Beacon token
            val serviceData = record.getServiceData(ParcelUuid(serviceUuid))
            if (serviceData != null && serviceData.contentEquals(token)) {
                // Connect to peer GATT
                connectToPeer(result.device)
            }
        }
    }

    private fun connectToPeer(device: BluetoothDevice) {
        device.connectGatt(context, false, object : BluetoothGattCallback() {
            override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
                if (newState == BluetoothProfile.STATE_CONNECTED) {
                    gatt.discoverServices()
                }
            }

            override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
                val service = gatt.getService(serviceUuid)
                val char = service?.getCharacteristic(charUuid)
                if (char != null) {
                    gatt.setCharacteristicNotification(char, true)
                    // Flush pending outgoing chunks
                    for (chunk in pendingChunks) {
                        char.value = chunk
                        char.writeType = BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
                        gatt.writeCharacteristic(char)
                    }
                }
            }

            override fun onCharacteristicChanged(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic) {
                characteristic.value?.let { handleIncomingChunk(it) }
            }
        })
    }

    private fun handleIncomingChunk(bytes: ByteArray) {
        try {
            val envelope = reassembler.pushChunk(bytes)
            if (envelope != null) {
                onEnvelopeReceived(envelope)
            }
        } catch (e: Exception) {
            Log.e(TAG, "BLE chunk reassembly error", e)
        }
    }

    companion object {
        private const val TAG = "BleSyncManager"
    }
}
