import Foundation
import CoreBluetooth
import Combine

/// Manages Bluetooth Low Energy (BLE) discovery, advertising, and peer-to-peer clip transmission.
/// Operates simultaneously as a Peripheral (broadcasting and serving clips) and Central (scanning and reading clips).
final class BleSyncManager: NSObject, ObservableObject {
    @Published var isBleAvailable: Bool = false
    @Published var nearbyPeersCount: Int = 0

    private var centralManager: CBCentralManager!
    private var peripheralManager: CBPeripheralManager!

    private var serviceUUID: CBUUID!
    private var charUUID: CBUUID!
    private var beaconToken: Data?

    private var characteristic: CBMutableCharacteristic?
    private var discoveredPeripherals = [UUID: CBPeripheral]()
    private var connectedPeripherals = [UUID: CBPeripheral]()
    private var pendingOutgoingChunks = [Data]()

    private let reassembler = FfiBleReassembler()
    private var onEnvelopeReceived: ((FfiEnvelope) -> Void)?

    init(onEnvelopeReceived: @escaping (FfiEnvelope) -> Void) {
        self.onEnvelopeReceived = onEnvelopeReceived
        super.init()

        let serviceStr = ffiBleServiceUuid()
        let charStr = ffiBleCharUuid()
        self.serviceUUID = CBUUID(string: serviceStr)
        self.charUUID = CBUUID(string: charStr)

        self.centralManager = CBCentralManager(delegate: self, queue: nil)
        self.peripheralManager = CBPeripheralManager(delegate: self, queue: nil)
    }

    func updateGroupKey(_ key: FfiGroupKey) {
        let tokenBytes = ffiBleBeaconToken(key: key)
        self.beaconToken = Data(tokenBytes)
        restartAdvertisingIfReady()
        restartScanningIfReady()
    }

    // MARK: - Outgoing Envelope Broadcasting via BLE
    func broadcastEnvelope(_ envelope: FfiEnvelope) {
        guard isBleAvailable else { return }
        do {
            // Chunk envelope into 490B payloads (standard BLE safe chunking)
            let chunks = try ffiBleChunkEnvelope(envelope: envelope, maxPayloadSize: 490)
            self.pendingOutgoingChunks = chunks

            // 1. Notify all subscribed centrals if we are acting as Peripheral
            if let char = characteristic {
                for chunk in chunks {
                    peripheralManager.updateValue(chunk, for: char, onSubscribedCentrals: nil)
                }
            }

            // 2. Write to all connected peripherals if we are acting as Central
            for (_, peripheral) in connectedPeripherals {
                if let service = peripheral.services?.first(where: { $0.uuid == serviceUUID }),
                   let char = service.characteristics?.first(where: { $0.uuid == charUUID }) {
                    for chunk in chunks {
                        peripheral.writeValue(chunk, for: char, type: .withoutResponse)
                    }
                }
            }
        } catch {
            print("BleSyncManager: failed to chunk envelope:", error)
        }
    }

    // MARK: - Peripheral Management
    private func restartAdvertisingIfReady() {
        guard peripheralManager.state == .poweredOn, beaconToken != nil else { return }
        peripheralManager.stopAdvertising()

        // Setup GATT Service & Characteristic
        let char = CBMutableCharacteristic(
            type: charUUID,
            properties: [.writeWithoutResponse, .notify, .read],
            value: nil,
            permissions: [.readable, .writeable]
        )
        self.characteristic = char

        let service = CBMutableService(type: serviceUUID, primary: true)
        service.characteristics = [char]

        peripheralManager.removeAllServices()
        peripheralManager.add(service)

        // Advertise with custom service UUID and local name
        // Note: macOS CBPeripheralManager only supports CBAdvertisementDataServiceUUIDsKey and CBAdvertisementDataLocalNameKey
        let advertisementData: [String: Any] = [
            CBAdvertisementDataServiceUUIDsKey: [serviceUUID!],
            CBAdvertisementDataLocalNameKey: "ClipBridge"
        ]
        peripheralManager.startAdvertising(advertisementData)
    }

    // MARK: - Central Management
    private func restartScanningIfReady() {
        guard centralManager.state == .poweredOn else { return }
        centralManager.stopScan()
        centralManager.scanForPeripherals(
            withServices: [serviceUUID],
            options: [CBCentralManagerScanOptionAllowDuplicatesKey: false]
        )
    }

    // MARK: - Incoming Chunk Handling
    private func handleIncomingChunkData(_ data: Data) {
        do {
            if let envelope = try reassembler.pushChunk(chunkBytes: data) {
                DispatchQueue.main.async { [weak self] in
                    self?.onEnvelopeReceived?(envelope)
                }
            }
        } catch {
            print("BleSyncManager: chunk error:", error)
        }
    }
}

// MARK: - CBCentralManagerDelegate
extension BleSyncManager: CBCentralManagerDelegate {
    func centralManagerDidUpdateState(_ central: CBCentralManager) {
        if central.state == .poweredOn {
            self.isBleAvailable = true
            restartScanningIfReady()
        } else {
            self.isBleAvailable = false
        }
    }

    func centralManager(_ central: CBCentralManager, didDiscover peripheral: CBPeripheral, advertisementData: [String : Any], rssi RSSI: NSNumber) {
        // Validate Beacon Token matches our Group Key (prevents connecting to unrelated devices)
        if let token = beaconToken,
           let serviceData = advertisementData[CBAdvertisementDataServiceDataKey] as? [CBUUID: Data],
           let peerToken = serviceData[serviceUUID] {
            guard peerToken == token else { return }
        }

        discoveredPeripherals[peripheral.identifier] = peripheral
        peripheral.delegate = self
        central.connect(peripheral, options: nil)
    }

    func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        connectedPeripherals[peripheral.identifier] = peripheral
        DispatchQueue.main.async { [weak self] in
            self?.nearbyPeersCount = self?.connectedPeripherals.count ?? 0
        }
        peripheral.discoverServices([serviceUUID])
    }

    func centralManager(_ central: CBCentralManager, didDisconnectPeripheral peripheral: CBPeripheral, error: Error?) {
        connectedPeripherals.removeValue(forKey: peripheral.identifier)
        DispatchQueue.main.async { [weak self] in
            self?.nearbyPeersCount = self?.connectedPeripherals.count ?? 0
        }
    }
}

// MARK: - CBPeripheralDelegate
extension BleSyncManager: CBPeripheralDelegate {
    func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
        guard let services = peripheral.services else { return }
        for service in services where service.uuid == serviceUUID {
            peripheral.discoverCharacteristics([charUUID], for: service)
        }
    }

    func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
        guard let characteristics = service.characteristics else { return }
        for char in characteristics where char.uuid == charUUID {
            // Subscribe to notifications for incoming chunks from peripheral
            peripheral.setNotifyValue(true, for: char)

            // If we have pending chunks to send, flush them to this peer
            for chunk in pendingOutgoingChunks {
                peripheral.writeValue(Data(chunk), for: char, type: .withoutResponse)
            }
        }
    }

    func peripheral(_ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?) {
        guard let data = characteristic.value else { return }
        handleIncomingChunkData(data)
    }
}

// MARK: - CBPeripheralManagerDelegate
extension BleSyncManager: CBPeripheralManagerDelegate {
    func peripheralManagerDidUpdateState(_ peripheral: CBPeripheralManager) {
        if peripheral.state == .poweredOn {
            restartAdvertisingIfReady()
        }
    }

    func peripheralManager(_ peripheral: CBPeripheralManager, didReceiveWrite requests: [CBATTRequest]) {
        for request in requests {
            if let data = request.value {
                handleIncomingChunkData(data)
                peripheral.respond(to: request, withResult: .success)
            }
        }
    }
}
