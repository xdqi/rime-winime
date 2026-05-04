use windows::core::{Interface, HSTRING, PCWSTR};
use windows::Win32::Foundation::{E_NOINTERFACE, E_NOTIMPL, HMODULE};
use windows::Win32::System::Com::{CoCreateInstance, IClassFactory, CLSCTX_INPROC_SERVER};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::TextServices::{
    ITfClientId, ITfCompartmentMgr, ITfKeystrokeMgr, ITfLangBarItemMgr, ITfMessagePump, ITfSource,
    ITfSourceSingle, ITfTextInputProcessor, ITfTextInputProcessorEx, ITfThreadMgr, ITfThreadMgrEx,
    ITfUIElementMgr, TF_TMAE_COMLESS,
};

use super::diag::{tsf_step, tsf_warn};
use super::registry::{guid_to_registry_string, TipInfo};

type PFnDllGetClassObject = unsafe extern "system" fn(
    rclsid: *const windows::core::GUID,
    riid: *const windows::core::GUID,
    ppv: *mut *mut core::ffi::c_void,
) -> windows::core::HRESULT;

/// Holds the loaded TIP DLL and its activated interface.
pub struct LoadedTip {
    pub tip: ITfTextInputProcessor,
    pub tip_ex: Option<ITfTextInputProcessorEx>,
    /// `HMODULE` from `LoadLibraryW` when using DllGetClassObject; otherwise null (CoCreate path).
    pub h_module: HMODULE,
    /// Whether the TIP was actually activated (at least one Activate/ActivateEx succeeded).
    pub activated: bool,
}

pub fn log_thread_mgr_surface(thread_mgr: &ITfThreadMgr, reason: &str) {
    let mut available = Vec::new();
    let mut missing = Vec::new();

    macro_rules! probe {
        ($t:ty, $name:expr) => {
            if thread_mgr.cast::<$t>().is_ok() {
                available.push($name);
            } else {
                missing.push($name);
            }
        };
    }

    probe!(ITfThreadMgr, "ITfThreadMgr");
    probe!(ITfThreadMgrEx, "ITfThreadMgrEx");
    probe!(ITfKeystrokeMgr, "ITfKeystrokeMgr");
    probe!(ITfSource, "ITfSource");
    probe!(ITfSourceSingle, "ITfSourceSingle");
    probe!(ITfCompartmentMgr, "ITfCompartmentMgr");
    probe!(ITfLangBarItemMgr, "ITfLangBarItemMgr");
    probe!(ITfMessagePump, "ITfMessagePump");
    probe!(ITfClientId, "ITfClientId");
    probe!(ITfUIElementMgr, "ITfUIElementMgr");

    tsf_step(format!(
        "[tsf] thread_mgr surface @{} available=[{}] missing=[{}]",
        reason,
        available.join(", "),
        missing.join(", ")
    ));
}

/// Try ActivateEx with the given flag order, then fall back to `Activate`.
/// Returns `true` if the TIP was actually activated (not E_NOTIMPL).
unsafe fn activate_with_order(
    tip: &ITfTextInputProcessor,
    tip_ex: Option<&ITfTextInputProcessorEx>,
    thread_mgr: &ITfThreadMgr,
    client_id: u32,
    activate_ex_attempts: &[(&str, u32)],
) -> Result<bool, String> {
    tsf_step(format!(
        "[tsf] activate_with_order: client_id={} activate_ex_steps={}",
        client_id,
        activate_ex_attempts.len()
    ));
    log_thread_mgr_surface(thread_mgr, "activate_with_order.begin");
    if let Some(ex) = tip_ex {
        let mut last_err: Option<windows::core::Error> = None;
        for (label, flags) in activate_ex_attempts {
            match unsafe { ex.ActivateEx(thread_mgr, client_id, *flags) } {
                Ok(()) => {
                    tsf_step(format!(
                        "[tsf] TIP activated via {} (flags=0x{:X})",
                        label, flags
                    ));
                    tsf_step("[tsf] activate_with_order: done (activated=true)");
                    return Ok(true);
                }
                Err(e) => {
                    if e.code() == E_NOINTERFACE {
                        tsf_warn(format!(
                            "[tsf] {}: E_NOINTERFACE; probing next activation path",
                            label
                        ));
                        log_thread_mgr_surface(thread_mgr, label);
                        last_err = Some(e);
                        continue;
                    }
                    if e.code() != E_NOTIMPL {
                        log_thread_mgr_surface(thread_mgr, label);
                        return Err(format!("{}: {:?}", label, e));
                    }
                    tsf_warn(format!("[tsf] {}: E_NOTIMPL (try next)", label));
                    last_err = Some(e);
                }
            }
        }
        if let Some(last) = last_err {
            tsf_warn(format!(
                "[tsf] All ActivateEx paths returned E_NOTIMPL ({:?}); trying Activate",
                last
            ));
            match unsafe { tip.Activate(thread_mgr, client_id) } {
                Ok(()) => {
                    tsf_step("[tsf] TIP activated via Activate (fallback)");
                    tsf_step("[tsf] activate_with_order: done (activated=true)");
                    return Ok(true);
                }
                Err(e) if e.code() == E_NOTIMPL || e.code() == E_NOINTERFACE => {
                    tsf_warn(
                        "[tsf] Activate returned E_NOTIMPL/E_NOINTERFACE; continuing without full TIP activation",
                    );
                    log_thread_mgr_surface(thread_mgr, "Activate fallback");
                }
                Err(e) => {
                    log_thread_mgr_surface(thread_mgr, "Activate fallback");
                    return Err(format!(
                        "ITfTextInputProcessor::Activate after ActivateEx: {:?}",
                        e
                    ));
                }
            }
        }
    } else {
        match unsafe { tip.Activate(thread_mgr, client_id) } {
            Ok(()) => {
                tsf_step("[tsf] TIP activated via Activate (no Ex)");
                tsf_step("[tsf] activate_with_order: done (activated=true)");
                return Ok(true);
            }
            Err(e) if e.code() == E_NOTIMPL || e.code() == E_NOINTERFACE => {
                tsf_warn(
                    "[tsf] Activate E_NOTIMPL/E_NOINTERFACE (no Ex); continuing without full TIP activation",
                );
                log_thread_mgr_surface(thread_mgr, "Activate(no Ex)");
            }
            Err(e) => {
                log_thread_mgr_surface(thread_mgr, "Activate(no Ex)");
                return Err(format!("ITfTextInputProcessor::Activate: {:?}", e));
            }
        }
    }
    tsf_step("[tsf] activate_with_order: done (activated=false)");
    Ok(false)
}

/// Load and activate a TIP: prefer `CoCreateInstance` (standard COM), then COM-less `DllGetClassObject`.
pub unsafe fn load_and_activate_tip(
    tip_info: &TipInfo,
    thread_mgr: &ITfThreadMgr,
    client_id: u32,
) -> Result<LoadedTip, String> {
    tsf_step(format!(
        "[tsf] load_and_activate_tip: begin clsid={} dll={} client_id={}",
        guid_to_registry_string(&tip_info.clsid),
        tip_info.dll_path,
        client_id
    ));

    // Standard in-proc activation (same as system TIP load). WeType often expects this path.
    let co_ex =
        CoCreateInstance::<_, ITfTextInputProcessorEx>(&tip_info.clsid, None, CLSCTX_INPROC_SERVER);
    if let Ok(ex) = co_ex {
        let tip = ex
            .cast::<ITfTextInputProcessor>()
            .map_err(|e| format!("QI(ITfTextInputProcessor) from CoCreate Ex: {:?}", e))?;
        tsf_step(
            "[tsf] TIP via CoCreateInstance(ITfTextInputProcessorEx), calling activate_with_order",
        );
        let activated = activate_with_order(
            &tip,
            Some(&ex),
            thread_mgr,
            client_id,
            &[
                ("ActivateEx(0)", 0u32),
                ("ActivateEx(COMLESS)", TF_TMAE_COMLESS),
            ],
        )?;
        tsf_step("[tsf] load_and_activate_tip: done (CoCreate Ex path)");
        return Ok(LoadedTip {
            tip,
            tip_ex: Some(ex),
            h_module: HMODULE::default(),
            activated,
        });
    }
    tsf_step(format!(
        "[tsf] CoCreateInstance(ITfTextInputProcessorEx): {:?}, trying ITfTextInputProcessor",
        co_ex.err()
    ));

    let co_tip =
        CoCreateInstance::<_, ITfTextInputProcessor>(&tip_info.clsid, None, CLSCTX_INPROC_SERVER);
    if let Ok(tip) = co_tip {
        let tip_ex = tip.cast::<ITfTextInputProcessorEx>().ok();
        tsf_step(
            "[tsf] TIP via CoCreateInstance(ITfTextInputProcessor), calling activate_with_order",
        );
        let activated = activate_with_order(
            &tip,
            tip_ex.as_ref(),
            thread_mgr,
            client_id,
            &[
                ("ActivateEx(0)", 0u32),
                ("ActivateEx(COMLESS)", TF_TMAE_COMLESS),
            ],
        )?;
        tsf_step("[tsf] load_and_activate_tip: done (CoCreate base path)");
        return Ok(LoadedTip {
            tip,
            tip_ex,
            h_module: HMODULE::default(),
            activated,
        });
    }
    tsf_step(format!(
        "[tsf] CoCreateInstance(ITfTextInputProcessor): {:?}; DllGetClassObject path",
        co_tip.err()
    ));

    tsf_warn("[tsf] CoCreateInstance TIP failed; loading DLL and using DllGetClassObject");

    tsf_step(format!("[tsf] LoadLibraryW: {}", tip_info.dll_path));
    let dll_hstring = HSTRING::from(tip_info.dll_path.as_str());
    let h_module = LoadLibraryW(PCWSTR::from_raw(dll_hstring.as_ptr()))
        .map_err(|e| format!("Failed to load TIP DLL '{}': {:?}", tip_info.dll_path, e))?;

    tsf_step(format!(
        "[tsf] LoadLibraryW ok: {} h_module={:?}",
        tip_info.dll_path, h_module
    ));

    tsf_step("[tsf] GetProcAddress(DllGetClassObject)");
    let fn_ptr = GetProcAddress(h_module, windows::core::s!("DllGetClassObject"))
        .ok_or_else(|| "DllGetClassObject export not found in TIP DLL".to_string())?;

    let dll_get_class_object: PFnDllGetClassObject = core::mem::transmute(fn_ptr);

    tsf_step("[tsf] DllGetClassObject(IClassFactory)");
    let mut factory_ptr: *mut core::ffi::c_void = core::ptr::null_mut();
    let hr = dll_get_class_object(&tip_info.clsid, &IClassFactory::IID, &mut factory_ptr);
    if hr.is_err() || factory_ptr.is_null() {
        return Err(format!(
            "DllGetClassObject failed: hr=0x{:08X}",
            hr.0 as u32
        ));
    }

    let factory: IClassFactory = unsafe { IClassFactory::from_raw(factory_ptr) };

    tsf_step("[tsf] IClassFactory ok, CreateInstance(ITfTextInputProcessorEx)");

    let (tip, tip_ex) = match factory.CreateInstance::<_, ITfTextInputProcessorEx>(None) {
        Ok(ex) => {
            let base = ex
                .cast::<ITfTextInputProcessor>()
                .map_err(|e| format!("QI(ITfTextInputProcessor) from Ex: {:?}", e))?;
            tsf_step("[tsf] CreateInstance(ITfTextInputProcessorEx) OK");
            (base, Some(ex))
        }
        Err(e_ex) => {
            tsf_warn(format!(
                "[tsf] CreateInstance(ITfTextInputProcessorEx): {:?}, trying base IFace",
                e_ex
            ));
            let tip = factory
                .CreateInstance::<_, ITfTextInputProcessor>(None)
                .map_err(|e| format!("CreateInstance(ITfTextInputProcessor): {:?}", e))?;
            let tip_ex = tip.cast::<ITfTextInputProcessorEx>().ok();
            (tip, tip_ex)
        }
    };

    // COM-less factory activation: prefer COMLESS, then plain ActivateEx(0).
    tsf_step("[tsf] DllGetClassObject path: activate_with_order");
    let activated = activate_with_order(
        &tip,
        tip_ex.as_ref(),
        thread_mgr,
        client_id,
        &[
            ("ActivateEx(COMLESS)", TF_TMAE_COMLESS),
            ("ActivateEx(0)", 0u32),
        ],
    )?;

    tsf_step("[tsf] load_and_activate_tip: done (DllGetClassObject path)");
    Ok(LoadedTip {
        tip,
        tip_ex,
        h_module,
        activated,
    })
}
