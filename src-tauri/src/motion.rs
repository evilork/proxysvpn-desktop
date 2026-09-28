// src-tauri/src/motion.rs
//
// Device tilt for the eye on the emblem.
//
// On iPhone and iPad the eye follows the tilt of the device the way it follows
// the mouse on the Mac. The angles come from CoreMotion through
// gen/apple/Sources/proxysvpn-desktop/MotionBridge.swift - natively, because
// the web's DeviceOrientationEvent needs a permission prompt inside a web view,
// and a prompt for a decoration is what App Review rejects (5.1.1). Device
// motion (gravity) needs no permission at all.
//
// The web view asks for readings with `motion_start` while the eye is on
// screen and releases them with `motion_stop`. In between a sampler thread
// reads the sensor about 30 times a second and emits `gaze-tilt` when the angle
// moved, plus a heartbeat every 200 ms: src/gaze.ts slowly re-centres the eye
// on the current pose over time, and a phone lying perfectly still would
// otherwise stop that clock. Nothing is stored and nothing leaves the device.
//
// On every other platform both commands are no-ops and `motion_start` answers
// false: the Mac has no tilt sensor, and the eye stays on the pointer.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
#[cfg(target_os = "ios")]
use std::time::Duration;

use serde::Serialize;

/// Event the web view listens to (src/bridge.ts).
pub const EVENT: &str = "gaze-tilt";

/// Below this change in either angle a new reading is not worth an event: the
/// eye's own travel is 22 degrees end to end (src/gaze.ts), so 0.15 degrees is
/// well under a pixel of pupil movement.
const MIN_CHANGE_DEG: f64 = 0.15;

#[cfg(target_os = "ios")]
const SAMPLE_EVERY: Duration = Duration::from_millis(33);

/// A still device still reports this often (see the header).
#[cfg(target_os = "ios")]
const HEARTBEAT: Duration = Duration::from_millis(200);

/// Tilt in DeviceOrientationEvent terms, degrees.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Tilt {
    /// Front to back: 0 lying flat face up, +90 upright facing the person.
    pub beta: f64,
    /// Left to right: positive when the right edge goes down.
    pub gamma: f64,
}

/// Which sampler is the live one. Every start and stop bumps it; a sampler
/// thread keeps running only while the number it was born with is current, so
/// a quick stop-start never leaves two threads emitting.
#[derive(Default)]
pub struct MotionState {
    generation: Arc<AtomicU64>,
}

/// Whether a reading moved far enough from the last emitted one to send.
fn changed_enough(prev: Option<Tilt>, next: Tilt) -> bool {
    match prev {
        None => true,
        Some(p) => {
            (next.beta - p.beta).abs() >= MIN_CHANGE_DEG
                || (next.gamma - p.gamma).abs() >= MIN_CHANGE_DEG
        }
    }
}

#[cfg(target_os = "ios")]
mod sensor {
    use super::Tilt;

    extern "C" {
        // Implemented in MotionBridge.swift.
        fn pvpn_motion_start() -> i32;
        fn pvpn_motion_stop();
        fn pvpn_motion_read(beta: *mut f64, gamma: *mut f64) -> i32;
    }

    pub fn start() -> bool {
        // SAFETY: plain C call into the Swift bridge, no pointers involved.
        unsafe { pvpn_motion_start() == 1 }
    }

    pub fn stop() {
        // SAFETY: plain C call into the Swift bridge, no pointers involved.
        unsafe { pvpn_motion_stop() }
    }

    pub fn read() -> Option<Tilt> {
        let mut beta = 0.0_f64;
        let mut gamma = 0.0_f64;
        // SAFETY: both pointers are valid, aligned locals that outlive the
        // call; the bridge writes them only when it returns 1.
        let ok = unsafe { pvpn_motion_read(&mut beta, &mut gamma) } == 1;
        (ok && beta.is_finite() && gamma.is_finite()).then_some(Tilt { beta, gamma })
    }
}

/// Start tilt readings. `false` - this device has no tilt sensor.
#[tauri::command]
pub fn motion_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, MotionState>,
) -> Result<bool, String> {
    let generation = Arc::clone(&state.generation);
    let mine = generation.fetch_add(1, Ordering::AcqRel) + 1;
    start_sampler(app, generation, mine)
}

/// Stop tilt readings. Safe to call when nothing runs.
#[tauri::command]
pub fn motion_stop(state: tauri::State<'_, MotionState>) -> Result<(), String> {
    state.generation.fetch_add(1, Ordering::AcqRel);
    #[cfg(target_os = "ios")]
    sensor::stop();
    Ok(())
}

#[cfg(target_os = "ios")]
fn start_sampler(
    app: tauri::AppHandle,
    generation: Arc<AtomicU64>,
    mine: u64,
) -> Result<bool, String> {
    use tauri::Emitter;

    if !sensor::start() {
        return Ok(false);
    }
    std::thread::Builder::new()
        .name("gaze-tilt".into())
        .spawn(move || {
            let mut last: Option<Tilt> = None;
            let mut sent_at = std::time::Instant::now();
            while generation.load(Ordering::Acquire) == mine {
                if let Some(tilt) = sensor::read() {
                    if changed_enough(last, tilt) || sent_at.elapsed() >= HEARTBEAT {
                        if let Err(err) = app.emit(EVENT, tilt) {
                            // The window is gone; nobody is left to watch.
                            crate::logger::log("warn", "motion", &format!("tilt emit failed: {err}"));
                            break;
                        }
                        last = Some(tilt);
                        sent_at = std::time::Instant::now();
                    }
                }
                std::thread::sleep(SAMPLE_EVERY);
            }
        })
        .map_err(|err| format!("tilt sampler did not start: {err}"))?;
    Ok(true)
}

#[cfg(not(target_os = "ios"))]
fn start_sampler(
    _app: tauri::AppHandle,
    _generation: Arc<AtomicU64>,
    _mine: u64,
) -> Result<bool, String> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tilt(beta: f64, gamma: f64) -> Tilt {
        Tilt { beta, gamma }
    }

    #[test]
    fn the_first_reading_is_always_sent() {
        assert!(changed_enough(None, tilt(0.0, 0.0)));
    }

    #[test]
    fn a_still_device_sends_nothing() {
        assert!(!changed_enough(Some(tilt(40.0, -3.0)), tilt(40.1, -3.05)));
    }

    #[test]
    fn a_move_on_either_axis_is_sent() {
        assert!(changed_enough(Some(tilt(40.0, -3.0)), tilt(40.2, -3.0)));
        assert!(changed_enough(Some(tilt(40.0, -3.0)), tilt(40.0, -3.2)));
        assert!(changed_enough(Some(tilt(40.0, -3.0)), tilt(39.8, -3.0)));
    }

    #[test]
    fn the_wire_format_is_what_the_web_view_reads() {
        let json = serde_json::to_value(tilt(12.5, -4.0)).expect("serialise");
        assert_eq!(json, serde_json::json!({ "beta": 12.5, "gamma": -4.0 }));
    }
}
