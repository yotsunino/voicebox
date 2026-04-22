//! Captures the focused Accessibility element at chord-start so auto-paste
//! can land in the user's original text field even after focus drifts
//! during transcription / refinement.
//!
//! We don't try to refocus a specific `AXUIElement` on restore — many apps
//! expose complex focus hierarchies that don't respond consistently to
//! `AXUIElementSetAttributeValue(kAXFocusedUIElementAttribute, ...)`.
//! Activating the owning app via `NSRunningApplication.activateWithOptions:`
//! brings it forward with its last-focused field still focused, which is
//! what every well-behaved paste-buffer tool does and what users expect.
//!
//! PID + bundle id + AX role are all captured for diagnostics — the bundle
//! id lets step 6 (internal direct injection) detect "focus was inside
//! Voicebox itself" and short-circuit the synthetic-paste path.

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FocusSnapshot {
    pub pid: i32,
    pub bundle_id: Option<String>,
    pub role: Option<String>,
}

#[cfg(target_os = "macos")]
use core_foundation_sys::base::{kCFAllocatorDefault, CFRelease};
#[cfg(target_os = "macos")]
use core_foundation_sys::string::{
    kCFStringEncodingUTF8, CFStringCreateWithCString, CFStringGetCString, CFStringGetLength,
    CFStringRef,
};
#[cfg(target_os = "macos")]
use objc::runtime::Object;
#[cfg(target_os = "macos")]
use objc::{class, msg_send, sel, sel_impl};

#[cfg(target_os = "macos")]
type Id = *mut Object;

#[cfg(target_os = "macos")]
mod ffi {
    use core_foundation_sys::base::CFTypeRef;
    use core_foundation_sys::string::CFStringRef;

    pub type AXError = i32;
    pub const AX_ERROR_SUCCESS: AXError = 0;
    pub type AXUIElementRef = *const std::ffi::c_void;
    pub type Pid = i32;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        pub fn AXUIElementCreateSystemWide() -> AXUIElementRef;
        pub fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> AXError;
        pub fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut Pid) -> AXError;
    }
    // AX attribute keys are exposed as C macros that expand to CFSTR(...)
    // literals, not as linkable symbols — build the CFStrings at runtime
    // instead (see `cf_string_const` in focus_capture.rs).
}

#[cfg(target_os = "macos")]
struct AutoreleasePool {
    pool: Id,
}

#[cfg(target_os = "macos")]
impl AutoreleasePool {
    unsafe fn new() -> Self {
        let pool: Id = msg_send![class!(NSAutoreleasePool), alloc];
        let pool: Id = msg_send![pool, init];
        Self { pool }
    }
}

#[cfg(target_os = "macos")]
impl Drop for AutoreleasePool {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![self.pool, drain];
        }
    }
}

#[cfg(target_os = "macos")]
unsafe fn ns_string_to_rust(s: Id) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let bytes: *const i8 = msg_send![s, UTF8String];
    if bytes.is_null() {
        return None;
    }
    std::ffi::CStr::from_ptr(bytes)
        .to_str()
        .ok()
        .map(|x| x.to_owned())
}

/// Build a `+1` retained CFString from an ASCII constant. Caller owns the
/// returned reference and must `CFRelease` it. Used for AX attribute keys
/// (`"AXFocusedUIElement"`, `"AXRole"`) because those aren't exported as
/// linker symbols — Apple ships them as `CFSTR(...)` macros.
#[cfg(target_os = "macos")]
unsafe fn cf_string_const(s: &str) -> Option<CFStringRef> {
    let cstr = std::ffi::CString::new(s).ok()?;
    let result = CFStringCreateWithCString(kCFAllocatorDefault, cstr.as_ptr(), kCFStringEncodingUTF8);
    if result.is_null() {
        None
    } else {
        Some(result)
    }
}

#[cfg(target_os = "macos")]
unsafe fn cfstring_to_rust(s: CFStringRef) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let len = CFStringGetLength(s);
    if len == 0 {
        return Some(String::new());
    }
    // CFStringGetLength is in UTF-16 code units; UTF-8 can need up to 4
    // bytes per unit plus the trailing NUL.
    let max_bytes = (len * 4 + 1) as usize;
    let mut buf = vec![0u8; max_bytes];
    let ok = CFStringGetCString(
        s,
        buf.as_mut_ptr() as *mut i8,
        max_bytes as isize,
        kCFStringEncodingUTF8,
    );
    if ok == 0 {
        return None;
    }
    let cstr = std::ffi::CStr::from_ptr(buf.as_ptr() as *const i8);
    cstr.to_str().ok().map(|x| x.to_owned())
}

#[cfg(target_os = "macos")]
unsafe fn bundle_id_for_pid(pid: i32) -> Option<String> {
    let _pool = AutoreleasePool::new();
    let app: Id = msg_send![
        class!(NSRunningApplication),
        runningApplicationWithProcessIdentifier: pid
    ];
    if app.is_null() {
        return None;
    }
    let bundle: Id = msg_send![app, bundleIdentifier];
    ns_string_to_rust(bundle)
}

/// Read the system-wide focused UI element's PID, bundle id, and AX role.
///
/// Returns an error when no element is focused (e.g. Dock has focus) or
/// when Accessibility permission is missing — `AXUIElementCopyAttributeValue`
/// returns `-25204 kAXErrorAPIDisabled` in that case.
#[cfg(target_os = "macos")]
pub fn capture_focus() -> Result<FocusSnapshot, String> {
    use ffi::*;
    unsafe {
        let system_wide = AXUIElementCreateSystemWide();
        if system_wide.is_null() {
            return Err("AXUIElementCreateSystemWide returned null".into());
        }
        let _sys_guard = scopeguard::guard(system_wide, |e| {
            CFRelease(e as *const std::ffi::c_void)
        });

        let focused_attr = cf_string_const("AXFocusedUIElement")
            .ok_or("Failed to build AXFocusedUIElement CFString")?;
        let _focused_attr_guard =
            scopeguard::guard(focused_attr, |s| CFRelease(s as *const std::ffi::c_void));

        let mut focused: *const std::ffi::c_void = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            system_wide,
            focused_attr,
            &mut focused as *mut _,
        );
        if err != AX_ERROR_SUCCESS || focused.is_null() {
            return Err(format!(
                "No focused element (AXError {}). Verify Accessibility permission is granted and a focused text field exists.",
                err
            ));
        }
        let _focus_guard = scopeguard::guard(focused, |e| CFRelease(e));

        let focused_elem = focused as AXUIElementRef;

        let mut pid: Pid = 0;
        let err = AXUIElementGetPid(focused_elem, &mut pid);
        if err != AX_ERROR_SUCCESS {
            return Err(format!("AXUIElementGetPid failed (AXError {})", err));
        }

        let role = {
            let role_attr = cf_string_const("AXRole");
            match role_attr {
                Some(role_attr) => {
                    let _role_attr_guard = scopeguard::guard(role_attr, |s| {
                        CFRelease(s as *const std::ffi::c_void)
                    });
                    let mut role_value: *const std::ffi::c_void = std::ptr::null();
                    let err = AXUIElementCopyAttributeValue(
                        focused_elem,
                        role_attr,
                        &mut role_value as *mut _,
                    );
                    if err == AX_ERROR_SUCCESS && !role_value.is_null() {
                        let _role_guard = scopeguard::guard(role_value, |e| CFRelease(e));
                        cfstring_to_rust(role_value as CFStringRef)
                    } else {
                        None
                    }
                }
                None => None,
            }
        };

        let bundle_id = bundle_id_for_pid(pid);

        Ok(FocusSnapshot {
            pid,
            bundle_id,
            role,
        })
    }
}

/// Bring the app owning `pid` to the foreground, re-activating its
/// last-focused window. Paired with [`capture_focus`] at chord-start so a
/// post-transcription synthetic ⌘V lands where the user started, not
/// wherever focus drifted to during the transcribe / refine window.
#[cfg(target_os = "macos")]
pub fn activate_pid(pid: i32) -> Result<(), String> {
    unsafe {
        let _pool = AutoreleasePool::new();
        let app: Id = msg_send![
            class!(NSRunningApplication),
            runningApplicationWithProcessIdentifier: pid
        ];
        if app.is_null() {
            return Err(format!("No running application for PID {}", pid));
        }
        // NSApplicationActivateIgnoringOtherApps = 1 << 1 = 2.
        //
        // macOS 14 deprecated this in favour of `activate()` but kept it
        // functional when the caller has Accessibility permission — which
        // we require for the paste event anyway.
        let _: bool = msg_send![app, activateWithOptions: 2u64];
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
pub fn capture_focus() -> Result<FocusSnapshot, String> {
    Err("focus capture is only implemented on macOS".into())
}

#[cfg(not(target_os = "macos"))]
pub fn activate_pid(_pid: i32) -> Result<(), String> {
    Err("app activation is only implemented on macOS".into())
}
