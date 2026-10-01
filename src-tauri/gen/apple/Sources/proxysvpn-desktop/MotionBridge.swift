// MotionBridge.swift
//
// Device tilt for the eye on the emblem, read natively through CoreMotion.
//
// Why not the web's DeviceOrientationEvent: inside WKWebView it needs a
// system permission prompt, and a prompt for a decoration is exactly what
// App Review frowns upon (5.1.1). CMMotionManager's device motion (gravity)
// needs no permission and no Info.plist usage string - that is reserved for
// activity and fitness data (CMMotionActivityManager, CMPedometer).
//
// Nothing here is stored or leaves the device: the Rust core reads two angles
// a few dozen times a second while the eye is on screen and hands them to the
// web view, and the sensor stops as soon as the eye is gone.
//
// Called from Rust (src-tauri/src/motion.rs) over C FFI, like VpnBridge.swift.

import CoreMotion
import Foundation

private final class TiltSensor {
    static let shared = TiltSensor()

    private let manager = CMMotionManager()
    // Start, stop and read arrive from Rust threads; CMMotionManager is not
    // documented as thread-safe, so every touch goes through one lock.
    private let lock = NSLock()

    func start() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard manager.isDeviceMotionAvailable else { return false }
        if !manager.isDeviceMotionActive {
            manager.deviceMotionUpdateInterval = 1.0 / 60.0
            // Pull mode: no handler and no queue - the core samples the
            // latest value at its own pace.
            manager.startDeviceMotionUpdates()
        }
        return true
    }

    func stop() {
        lock.lock()
        defer { lock.unlock() }
        if manager.isDeviceMotionActive {
            manager.stopDeviceMotionUpdates()
        }
    }

    /// Tilt in the same frame and sign convention as the web's
    /// DeviceOrientationEvent, so src/gaze.ts treats both sources alike:
    ///   beta  - front-to-back, 0 lying flat face up, +90 upright facing you;
    ///   gamma - left-to-right, positive when the right edge goes down.
    /// Derived from gravity (device axes: x right, y to the top edge, z out of
    /// the screen; gravity points to the ground, 1 g long).
    func read() -> (beta: Double, gamma: Double)? {
        lock.lock()
        defer { lock.unlock() }
        guard let g = manager.deviceMotion?.gravity else { return nil }
        let beta = atan2(-g.y, -g.z) * 180.0 / .pi
        let gamma = atan2(g.x, (g.y * g.y + g.z * g.z).squareRoot()) * 180.0 / .pi
        guard beta.isFinite, gamma.isFinite else { return nil }
        return (beta, gamma)
    }
}

/// 1 when the sensor runs (or already ran), 0 when the device has none.
@_cdecl("pvpn_motion_start")
public func pvpn_motion_start() -> Int32 {
    TiltSensor.shared.start() ? 1 : 0
}

@_cdecl("pvpn_motion_stop")
public func pvpn_motion_stop() {
    TiltSensor.shared.stop()
}

/// 1 and both angles in degrees when a reading exists; 0 before the first
/// sample, after stop, or on a device without the sensor.
@_cdecl("pvpn_motion_read")
public func pvpn_motion_read(
    _ beta: UnsafeMutablePointer<Double>?,
    _ gamma: UnsafeMutablePointer<Double>?
) -> Int32 {
    guard let beta = beta, let gamma = gamma, let tilt = TiltSensor.shared.read() else { return 0 }
    beta.pointee = tilt.beta
    gamma.pointee = tilt.gamma
    return 1
}
