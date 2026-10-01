// src-tauri/src/entropy.rs
//
// Bytes from the operating system's CSPRNG, for the four things that need
// them: the device id (device_id.rs), the per-install salt of the network
// memory (netmem.rs), the race inbound's password (xray_manager.rs) and the
// pair-code request id (pair_code.rs).
//
// Unix keeps reading /dev/urandom, exactly as those four did on their own: on
// macOS, iOS and Linux the device is always present and never blocks once the
// system is up. Windows has no such device, so there the same bytes come from
// the system RNG through `getrandom` (ProcessPrng / BCryptGenRandom).

/// Fill `buf` completely, or fail. A short read is an error, never zeros.
pub fn fill(buf: &mut [u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Read;
        // `read_exact` retries on EINTR and turns a short read into an error.
        std::fs::File::open("/dev/urandom")?.read_exact(buf)
    }

    #[cfg(windows)]
    {
        getrandom::fill(buf).map_err(|e| std::io::Error::other(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_the_whole_buffer_and_differs_between_calls() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        fill(&mut a).expect("system randomness");
        fill(&mut b).expect("system randomness");
        assert_ne!(a, [0u8; 32], "an all-zero draw means nothing was written");
        assert_ne!(a, b, "two draws must not collide");
    }

    #[test]
    fn an_empty_buffer_is_fine() {
        fill(&mut []).expect("nothing to fill");
    }
}
