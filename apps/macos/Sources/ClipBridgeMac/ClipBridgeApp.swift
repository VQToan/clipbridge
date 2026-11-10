import SwiftUI

@main
struct ClipBridgeApp: App {
    @StateObject private var engine = SyncEngine()

    var body: some Scene {
        MenuBarExtra("ClipBridge", systemImage: engine.state.systemIcon) {
            MenuBarView(engine: engine)
        }
        .menuBarExtraStyle(.window)
    }
}
