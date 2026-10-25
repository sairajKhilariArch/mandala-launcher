//! Stable C ABI between the Mandala host (`mandala-core`) and sub-modules.
//!
//! A module is a `cdylib` that exports exactly one symbol,
//! [`ENTRY_SYMBOL`] (`mandala_module_entry`), returning a pointer to a static
//! [`ModuleVTable`]. Module authors never write that by hand: implement the
//! [`Module`] trait and call [`declare_module!`].
//!
//! All strings crossing the boundary are NUL-terminated UTF-8. Strings
//! returned by a module are owned by the module and must be released with
//! `ModuleVTable::free_string`.

use serde::{Deserialize, Serialize};
use std::ffi::{c_char, CStr};

/// Bump on any breaking change to [`ModuleVTable`] or [`HostApi`].
pub const ABI_VERSION: u32 = 2;

/// Symbol looked up by the host loader (NUL-terminated for `libloading`).
pub const ENTRY_SYMBOL: &[u8] = b"mandala_module_entry\0";

/// Return codes of [`HostApi::send`].
pub const SEND_OK: i32 = 0;
pub const SEND_ACL_DENIED: i32 = 1;
pub const SEND_QUEUE_FULL: i32 = 2;
pub const SEND_BAD_ARGS: i32 = 3;

/// Callbacks the host hands to a module during `init`.
#[repr(C)]
pub struct HostApi {
    pub abi_version: u32,
    /// `send(sender_id, target, action, payload) -> SEND_*`
    ///
    /// Non-blocking. The message goes through the host's ACL-enforcing IPC
    /// broker, so a module can only reach targets it was granted.
    pub send: extern "C" fn(
        sender: *const c_char,
        target: *const c_char,
        action: *const c_char,
        payload: *const c_char,
    ) -> i32,
}

/// Function table every module exposes.
#[repr(C)]
pub struct ModuleVTable {
    pub abi_version: u32,
    pub id: extern "C" fn() -> *const c_char,
    pub name: extern "C" fn() -> *const c_char,
    /// Returns 0 on success.
    pub init: extern "C" fn(data_dir: *const c_char, host: *const HostApi) -> i32,
    /// Returns a JSON-encoded [`ModuleReply`]; free with `free_string`.
    pub handle: extern "C" fn(action: *const c_char, payload: *const c_char) -> *mut c_char,
    pub free_string: extern "C" fn(*mut c_char),
}

/// Signature of the exported entry symbol.
pub type EntryFn = unsafe extern "C" fn() -> *const ModuleVTable;

/// What a module answers to every request.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ModuleReply {
    pub ok: bool,
    pub text: String,
}

impl ModuleReply {
    pub fn ok(text: impl Into<String>) -> Self {
        Self {
            ok: true,
            text: text.into(),
        }
    }
    pub fn err(text: impl Into<String>) -> Self {
        Self {
            ok: false,
            text: text.into(),
        }
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(self)
            .unwrap_or_else(|_| r#"{"ok":false,"text":"serialize failure"}"#.to_string())
    }
    pub fn from_json(s: &str) -> Self {
        serde_json::from_str(s).unwrap_or_else(|e| Self::err(format!("bad module reply: {e}")))
    }
}

/// Handle a module uses to talk to other modules / the host.
#[derive(Clone, Copy)]
pub struct Host {
    api: *const HostApi,
    sender_id: &'static str,
}

// SAFETY: `HostApi` is a table of plain function pointers owned by the host
// for the process lifetime; calling them from any thread is allowed.
unsafe impl Send for Host {}
unsafe impl Sync for Host {}

impl Host {
    #[doc(hidden)]
    pub fn new(api: *const HostApi, sender_id: &'static str) -> Self {
        Self { api, sender_id }
    }

    /// A handle that rejects every message (used before `init` and in tests).
    pub fn detached(sender_id: &'static str) -> Self {
        Self {
            api: std::ptr::null(),
            sender_id,
        }
    }

    /// Send a fire-and-forget message through the host's IPC broker.
    pub fn send(&self, target: &str, action: &str, payload: &str) -> Result<(), i32> {
        if self.api.is_null() {
            return Err(SEND_BAD_ARGS);
        }
        let c = |s: &str| std::ffi::CString::new(s.replace('\0', "")).map_err(|_| SEND_BAD_ARGS);
        let (s, t, a, p) = (c(self.sender_id)?, c(target)?, c(action)?, c(payload)?);
        // SAFETY: api is non-null and valid for the process lifetime.
        let code = unsafe { ((*self.api).send)(s.as_ptr(), t.as_ptr(), a.as_ptr(), p.as_ptr()) };
        if code == SEND_OK {
            Ok(())
        } else {
            Err(code)
        }
    }
}

/// Context passed to [`Module::init`].
pub struct ModuleContext {
    /// Private, writable directory for this module.
    pub data_dir: String,
    pub host: Host,
}

/// Implement this and call [`declare_module!`].
pub trait Module: Send + 'static {
    /// Unique id; must match the file name the host loads (`<id>.so`).
    fn id(&self) -> &'static str;
    /// Human-readable name.
    fn name(&self) -> &'static str;
    fn init(&mut self, _ctx: &ModuleContext) -> Result<(), String> {
        Ok(())
    }
    /// Actions: `launch` is sent when the user opens the module's slot.
    fn handle(&mut self, action: &str, payload: &str) -> ModuleReply;
}

/// # Safety
/// `ptr` must be null or point to a valid NUL-terminated string.
#[doc(hidden)]
pub unsafe fn cstr_lossy(ptr: *const c_char) -> String {
    if ptr.is_null() {
        String::new()
    } else {
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

/// Export a [`Module`] implementation from a `cdylib` crate.
///
/// ```ignore
/// mandala_module_api::declare_module!(MyModule::new);
/// ```
/// Use once per crate. The constructor is `fn() -> impl Module`.
#[macro_export]
macro_rules! declare_module {
    ($ctor:expr) => {
        fn __mandala_instance() -> &'static ::std::sync::Mutex<::std::boxed::Box<dyn $crate::Module>>
        {
            static INSTANCE: ::std::sync::OnceLock<
                ::std::sync::Mutex<::std::boxed::Box<dyn $crate::Module>>,
            > = ::std::sync::OnceLock::new();
            INSTANCE.get_or_init(|| {
                let m: ::std::boxed::Box<dyn $crate::Module> = ::std::boxed::Box::new(($ctor)());
                ::std::sync::Mutex::new(m)
            })
        }

        extern "C" fn __mandala_id() -> *const ::std::ffi::c_char {
            static S: ::std::sync::OnceLock<::std::ffi::CString> = ::std::sync::OnceLock::new();
            S.get_or_init(|| {
                let g = __mandala_instance()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                ::std::ffi::CString::new(g.id()).unwrap_or_default()
            })
            .as_ptr()
        }

        extern "C" fn __mandala_name() -> *const ::std::ffi::c_char {
            static S: ::std::sync::OnceLock<::std::ffi::CString> = ::std::sync::OnceLock::new();
            S.get_or_init(|| {
                let g = __mandala_instance()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                ::std::ffi::CString::new(g.name()).unwrap_or_default()
            })
            .as_ptr()
        }

        extern "C" fn __mandala_init(
            data_dir: *const ::std::ffi::c_char,
            host: *const $crate::HostApi,
        ) -> i32 {
            let r = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                let dir = unsafe { $crate::cstr_lossy(data_dir) };
                let mut g = __mandala_instance()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let id: &'static str = g.id();
                let ctx = $crate::ModuleContext {
                    data_dir: dir,
                    host: $crate::Host::new(host, id),
                };
                g.init(&ctx)
            }));
            match r {
                Ok(Ok(())) => 0,
                Ok(Err(_)) => 1,
                Err(_) => 2,
            }
        }

        extern "C" fn __mandala_handle(
            action: *const ::std::ffi::c_char,
            payload: *const ::std::ffi::c_char,
        ) -> *mut ::std::ffi::c_char {
            let reply = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                let a = unsafe { $crate::cstr_lossy(action) };
                let p = unsafe { $crate::cstr_lossy(payload) };
                let mut g = __mandala_instance()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                g.handle(&a, &p)
            }))
            .unwrap_or_else(|_| $crate::ModuleReply::err("module panicked"));
            let json = reply.to_json().replace('\0', "");
            ::std::ffi::CString::new(json)
                .unwrap_or_default()
                .into_raw()
        }

        extern "C" fn __mandala_free(p: *mut ::std::ffi::c_char) {
            if !p.is_null() {
                drop(unsafe { ::std::ffi::CString::from_raw(p) });
            }
        }

        static __MANDALA_VTABLE: $crate::ModuleVTable = $crate::ModuleVTable {
            abi_version: $crate::ABI_VERSION,
            id: __mandala_id,
            name: __mandala_name,
            init: __mandala_init,
            handle: __mandala_handle,
            free_string: __mandala_free,
        };

        #[no_mangle]
        pub extern "C" fn mandala_module_entry() -> *const $crate::ModuleVTable {
            &__MANDALA_VTABLE
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_roundtrip() {
        let r = ModuleReply::ok("hi");
        assert_eq!(ModuleReply::from_json(&r.to_json()), r);
    }

    #[test]
    fn bad_reply_becomes_error() {
        assert!(!ModuleReply::from_json("not json").ok);
    }

    #[test]
    fn detached_host_rejects() {
        assert_eq!(Host::detached("x").send("y", "a", "p"), Err(SEND_BAD_ARGS));
    }
}
