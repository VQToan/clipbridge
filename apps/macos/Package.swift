// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "ClipBridgeMac",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "ClipBridgeMac", targets: ["ClipBridgeMac"]),
    ],
    targets: [
        .target(
            name: "clip_ffiFFI",
            path: "Sources/clip_ffiFFI",
            publicHeadersPath: "include"
        ),
        .executableTarget(
            name: "ClipBridgeMac",
            dependencies: ["clip_ffiFFI"],
            path: "Sources/ClipBridgeMac",
            linkerSettings: [
                .unsafeFlags([
                    "-L../../core/target/debug",
                    "-lclip_ffi",
                    "-Xlinker", "-rpath",
                    "-Xlinker", "@loader_path/../../../../core/target/debug",
                    "-Xlinker", "-rpath",
                    "-Xlinker", "/Users/toanvo/.gemini/antigravity/scratch/clipbridge/core/target/debug"
                ])
            ]
        ),
    ]
)
