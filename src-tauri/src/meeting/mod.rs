//! Native, provider-agnostic "is the user in a meeting?" probe (AC2-105).
//!
//! Meeting state = any process anywhere on the system is actively capturing the
//! mic or camera. We read the public CoreAudio / CoreMediaIO "is running
//! somewhere" properties — the same read-only queries Hand Mirror / OverSight
//! use, which need no entitlements and don't open the device (sub-10 ms). This
//! catches any conferencing tool (Zoom, Meet, Teams, Slack huddles, FaceTime…)
//! without per-app logic.
//!
//! macOS-only; a no-op (`false`) on every other OS. La Vigie owns this decision
//! for its own sounds — no marker files, no menu-bar app, no hook contract.

#[cfg(not(target_os = "macos"))]
pub fn is_meeting_active() -> bool {
    false
}

#[cfg(target_os = "macos")]
pub use imp::is_meeting_active;

#[cfg(target_os = "macos")]
mod imp {
    use std::os::raw::c_void;
    use std::ptr::null;

    /// Build a FourCC selector/scope constant the way the CoreAudio headers do.
    const fn fourcc(s: &[u8; 4]) -> u32 {
        ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | (s[3] as u32)
    }

    // Constants below are copied verbatim from the macOS SDK headers
    // (CoreAudio/AudioHardware*.h, CoreMediaIO/CMIOHardware*.h).
    const SYSTEM_OBJECT: u32 = 1; // kAudio/CMIOObjectSystemObject
    const PROP_DEVICES: u32 = fourcc(b"dev#"); // kAudio/CMIOHardwarePropertyDevices
    const PROP_RUNNING_SOMEWHERE: u32 = fourcc(b"gone"); // …DeviceIsRunningSomewhere
    const PROP_STREAMS: u32 = fourcc(b"stm#"); // kAudioDevicePropertyStreams
    const SCOPE_GLOBAL: u32 = fourcc(b"glob"); // kAudio/CMIOObjectPropertyScopeGlobal
    const SCOPE_INPUT: u32 = fourcc(b"inpt"); // kAudioDevicePropertyScopeInput
    const SCOPE_WILDCARD: u32 = fourcc(b"****"); // kCMIOObjectPropertyScopeWildcard
    const ELEMENT_MAIN: u32 = 0; // kAudio/CMIOObjectPropertyElementMain
    const ELEMENT_WILDCARD: u32 = 0xFFFF_FFFF; // kCMIOObjectPropertyElementWildcard

    /// Shared layout of AudioObjectPropertyAddress / CMIOObjectPropertyAddress
    /// (both are three packed `UInt32`s).
    #[repr(C)]
    struct PropertyAddress {
        selector: u32,
        scope: u32,
        element: u32,
    }

    #[link(name = "CoreAudio", kind = "framework")]
    extern "C" {
        fn AudioObjectGetPropertyDataSize(
            in_object_id: u32,
            in_address: *const PropertyAddress,
            in_qualifier_data_size: u32,
            in_qualifier_data: *const c_void,
            out_data_size: *mut u32,
        ) -> i32;

        fn AudioObjectGetPropertyData(
            in_object_id: u32,
            in_address: *const PropertyAddress,
            in_qualifier_data_size: u32,
            in_qualifier_data: *const c_void,
            io_data_size: *mut u32,
            out_data: *mut c_void,
        ) -> i32;
    }

    #[link(name = "CoreMediaIO", kind = "framework")]
    extern "C" {
        fn CMIOObjectGetPropertyDataSize(
            object_id: u32,
            address: *const PropertyAddress,
            qualifier_data_size: u32,
            qualifier_data: *const c_void,
            data_size: *mut u32,
        ) -> i32;

        fn CMIOObjectGetPropertyData(
            object_id: u32,
            address: *const PropertyAddress,
            qualifier_data_size: u32,
            qualifier_data: *const c_void,
            data_size: u32,
            data_used: *mut u32,
            data: *mut c_void,
        ) -> i32;
    }

    const U32_SIZE: u32 = std::mem::size_of::<u32>() as u32;

    /// All audio device IDs known to the system (empty on any CoreAudio error).
    fn audio_devices() -> Vec<u32> {
        let addr = PropertyAddress {
            selector: PROP_DEVICES,
            scope: SCOPE_GLOBAL,
            element: ELEMENT_MAIN,
        };
        let mut size: u32 = 0;
        // SAFETY: addr/size are valid; no qualifier needed for this property.
        if unsafe { AudioObjectGetPropertyDataSize(SYSTEM_OBJECT, &addr, 0, null(), &mut size) } != 0
        {
            return Vec::new();
        }
        let count = (size / U32_SIZE) as usize;
        if count == 0 {
            return Vec::new();
        }
        let mut ids = vec![0u32; count];
        let mut io = size;
        // SAFETY: ids has room for `count` u32s; io tracks the buffer size.
        if unsafe {
            AudioObjectGetPropertyData(
                SYSTEM_OBJECT,
                &addr,
                0,
                null(),
                &mut io,
                ids.as_mut_ptr() as *mut c_void,
            )
        } != 0
        {
            return Vec::new();
        }
        ids
    }

    /// Does this audio device expose input streams (i.e. is it a capture device)?
    /// We only treat input-capable devices as "mic", matching Hand Mirror.
    fn audio_has_input_streams(device: u32) -> bool {
        let addr = PropertyAddress {
            selector: PROP_STREAMS,
            scope: SCOPE_INPUT,
            element: ELEMENT_MAIN,
        };
        let mut size: u32 = 0;
        // SAFETY: addr/size valid; a non-zero size means input streams exist.
        let ok = unsafe { AudioObjectGetPropertyDataSize(device, &addr, 0, null(), &mut size) == 0 };
        ok && size > 0
    }

    /// Is this device's input IO running in any process right now?
    fn audio_running_somewhere(device: u32) -> bool {
        read_u32_property_audio(device, PROP_RUNNING_SOMEWHERE, SCOPE_INPUT, ELEMENT_MAIN) != 0
    }

    fn read_u32_property_audio(device: u32, selector: u32, scope: u32, element: u32) -> u32 {
        let addr = PropertyAddress {
            selector,
            scope,
            element,
        };
        let mut value: u32 = 0;
        let mut io = U32_SIZE;
        // SAFETY: value is a single u32 and io matches its size.
        let ok = unsafe {
            AudioObjectGetPropertyData(
                device,
                &addr,
                0,
                null(),
                &mut io,
                &mut value as *mut u32 as *mut c_void,
            ) == 0
        };
        if ok {
            value
        } else {
            0
        }
    }

    /// True if any capture-capable audio device is actively recording.
    fn mic_active() -> bool {
        audio_devices()
            .into_iter()
            .any(|d| audio_has_input_streams(d) && audio_running_somewhere(d))
    }

    /// All video capture device IDs known to CoreMediaIO (empty on any error).
    fn video_devices() -> Vec<u32> {
        let addr = PropertyAddress {
            selector: PROP_DEVICES,
            scope: SCOPE_GLOBAL,
            element: ELEMENT_MAIN,
        };
        let mut size: u32 = 0;
        // SAFETY: addr/size valid; CMIO size query takes no qualifier.
        if unsafe { CMIOObjectGetPropertyDataSize(SYSTEM_OBJECT, &addr, 0, null(), &mut size) } != 0
        {
            return Vec::new();
        }
        let count = (size / U32_SIZE) as usize;
        if count == 0 {
            return Vec::new();
        }
        let mut ids = vec![0u32; count];
        let mut used: u32 = 0;
        // SAFETY: ids holds `count` u32s; `size` is the byte capacity, `used`
        // receives the bytes written.
        if unsafe {
            CMIOObjectGetPropertyData(
                SYSTEM_OBJECT,
                &addr,
                0,
                null(),
                size,
                &mut used,
                ids.as_mut_ptr() as *mut c_void,
            )
        } != 0
        {
            return Vec::new();
        }
        ids
    }

    /// Is this camera capturing in any process right now? CMIO reports the
    /// running state under the wildcard scope/element for the device.
    fn camera_running_somewhere(device: u32) -> bool {
        let addr = PropertyAddress {
            selector: PROP_RUNNING_SOMEWHERE,
            scope: SCOPE_WILDCARD,
            element: ELEMENT_WILDCARD,
        };
        let mut value: u32 = 0;
        let mut used: u32 = 0;
        // SAFETY: value is one u32; U32_SIZE is its byte capacity.
        let ok = unsafe {
            CMIOObjectGetPropertyData(
                device,
                &addr,
                0,
                null(),
                U32_SIZE,
                &mut used,
                &mut value as *mut u32 as *mut c_void,
            ) == 0
        };
        ok && value != 0
    }

    /// True if any video capture device is actively capturing.
    fn camera_active() -> bool {
        video_devices().into_iter().any(camera_running_somewhere)
    }

    /// Mic OR camera in use anywhere on the system.
    pub fn is_meeting_active() -> bool {
        mic_active() || camera_active()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // Smoke test: the FFI probe runs headlessly on the macOS host. It must
        // not panic/hang and must return a real bool. (We can't assert the value
        // — it depends on whether the test machine has the mic/camera live.)
        #[test]
        fn probe_runs_without_panicking() {
            let _ = is_meeting_active();
        }

        // FourCC packing matches the byte order CoreAudio uses ('dev#').
        #[test]
        fn fourcc_packs_big_endian() {
            assert_eq!(fourcc(b"dev#"), 0x6465_7623);
            assert_eq!(fourcc(b"gone"), 0x676F_6E65);
            assert_eq!(fourcc(b"****"), 0x2A2A_2A2A);
        }
    }
}
