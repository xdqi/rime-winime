use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use windows::core::Interface;
use windows::core::VARIANT;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SetKeyboardState, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::TextServices::{
    CLSID_TF_InputProcessorProfiles, CLSID_TF_ThreadMgr, ITextStoreACP, ITfCompartmentMgr,
    ITfContext, ITfDocumentMgr, ITfInputProcessorProfiles, ITfKeystrokeMgr, ITfThreadMgr,
    GUID_COMPARTMENT_EMPTYCONTEXT, GUID_COMPARTMENT_KEYBOARD_DISABLED,
    GUID_COMPARTMENT_KEYBOARD_INPUTMODE, GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION,
    GUID_COMPARTMENT_KEYBOARD_INPUTMODE_SENTENCE, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
    GUID_TFCAT_TIP_KEYBOARD, TF_CONVERSIONMODE_NATIVE, TF_LANGUAGEPROFILE,
    TF_PROFILETYPE_INPUTPROCESSOR,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
};

pub mod diag;
pub mod registry;
pub mod text_store;
pub mod thread_mgr_wrap;
pub mod tip_loader;
pub mod tsf_candidates;

use crate::backend::RimeBackend;
use crate::proto::rime_service_v2::{KeyEvent, RimeContextProto};
use crate::win_keymap;
use diag::tsf_step;
use registry::{guid_to_registry_string, resolve_tip, TipInfo};
use text_store::{TextStoreAcp, TextStoreInner};
use thread_mgr_wrap::{acquire_lang_bar_item_mgr, ThreadMgrProxy, TsfHostState};
use tip_loader::load_and_activate_tip;

fn describe_compartment_guid(guid: &windows::core::GUID) -> &'static str {
    if *guid == GUID_COMPARTMENT_EMPTYCONTEXT {
        "GUID_COMPARTMENT_EMPTYCONTEXT"
    } else if *guid == GUID_COMPARTMENT_KEYBOARD_DISABLED {
        "GUID_COMPARTMENT_KEYBOARD_DISABLED"
    } else if *guid == GUID_COMPARTMENT_KEYBOARD_OPENCLOSE {
        "GUID_COMPARTMENT_KEYBOARD_OPENCLOSE"
    } else if *guid == GUID_COMPARTMENT_KEYBOARD_INPUTMODE {
        "GUID_COMPARTMENT_KEYBOARD_INPUTMODE"
    } else if *guid == GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION {
        "GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION"
    } else if *guid == GUID_COMPARTMENT_KEYBOARD_INPUTMODE_SENTENCE {
        "GUID_COMPARTMENT_KEYBOARD_INPUTMODE_SENTENCE"
    } else {
        "unknown"
    }
}

unsafe fn compartment_i32(mgr: &ITfCompartmentMgr, guid: &windows::core::GUID) -> Option<i32> {
    let compartment = mgr.GetCompartment(guid).ok()?;
    let value = compartment.GetValue().ok()?;
    Some(value.as_raw().Anonymous.Anonymous.Anonymous.lVal)
}

unsafe fn log_compartment_snapshot(label: &str, mgr: &ITfCompartmentMgr) {
    for guid in [
        GUID_COMPARTMENT_EMPTYCONTEXT,
        GUID_COMPARTMENT_KEYBOARD_DISABLED,
        GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
        GUID_COMPARTMENT_KEYBOARD_INPUTMODE,
        GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION,
        GUID_COMPARTMENT_KEYBOARD_INPUTMODE_SENTENCE,
    ] {
        tsf_step(format!(
            "[tsf] compartment {} {} ({:?}) = {:?}",
            label,
            describe_compartment_guid(&guid),
            guid,
            compartment_i32(mgr, &guid)
        ));
    }
}

struct CoInitGuard;

impl Drop for CoInitGuard {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

pub struct TsfRimeAdapter {
    _com: CoInitGuard,
    state: Arc<TsfHostState>,
    thread_mgr: ITfThreadMgr,
    client_id: u32,
    loaded_tip: tip_loader::LoadedTip,
    sessions: HashMap<usize, TsfSession>,
    next_id: usize,
}

struct TsfSession {
    doc_mgr: ITfDocumentMgr,
    #[allow(dead_code)]
    context: ITfContext,
    #[allow(dead_code)]
    text_store: ITextStoreACP,
    text: Arc<TextStoreInner>,
    pending_commit: Option<String>,
    last_preedit: String,
}

unsafe fn set_compartment_i4(
    mgr: &ITfCompartmentMgr,
    client_id: u32,
    guid: &windows::core::GUID,
    value: i32,
) {
    if let Ok(compartment) = mgr.GetCompartment(guid) {
        let var = VARIANT::from(value);
        let result = compartment.SetValue(client_id, &var);
        tsf_step(format!(
            "[tsf] set_compartment_i4 guid={:?} value={} -> {:?}",
            guid, value, result
        ));
    } else {
        tsf_step(format!(
            "[tsf] set_compartment_i4 guid={:?} value={} -> GetCompartment failed",
            guid, value
        ));
    }
}

unsafe fn initialize_keyboard_compartments(mgr: &ITfCompartmentMgr, client_id: u32) {
    set_compartment_i4(mgr, client_id, &GUID_COMPARTMENT_KEYBOARD_DISABLED, 0);
    set_compartment_i4(mgr, client_id, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE, 1);
    set_compartment_i4(
        mgr,
        client_id,
        &GUID_COMPARTMENT_KEYBOARD_INPUTMODE,
        TF_CONVERSIONMODE_NATIVE as i32,
    );
    set_compartment_i4(
        mgr,
        client_id,
        &GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION,
        TF_CONVERSIONMODE_NATIVE as i32,
    );
    set_compartment_i4(
        mgr,
        client_id,
        &GUID_COMPARTMENT_KEYBOARD_INPUTMODE_SENTENCE,
        0,
    );
}

unsafe fn sync_input_processor_profile(tip_info: &mut TipInfo) {
    let profiles: ITfInputProcessorProfiles =
        match CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER) {
            Ok(p) => p,
            Err(e) => {
                tsf_step(format!(
                    "[tsf] input profiles unavailable for sync: {:?}",
                    e
                ));
                return;
            }
        };

    let current_lang = profiles.GetCurrentLanguage();
    tsf_step(format!(
        "[tsf] input profiles current_lang before sync={:?} target=0x{:04X}",
        current_lang, tip_info.lang_id
    ));

    if current_lang.ok() != Some(tip_info.lang_id) {
        let result = profiles.ChangeCurrentLanguage(tip_info.lang_id);
        tsf_step(format!(
            "[tsf] input profiles ChangeCurrentLanguage(0x{:04X}) -> {:?}",
            tip_info.lang_id, result
        ));
        if result.is_ok() && tip_info.profile_guid == windows::core::GUID::zeroed() {
            fill_tip_profile_from_input_profiles(tip_info);
        }
    }

    if tip_info.profile_guid == windows::core::GUID::zeroed() {
        tsf_step(format!(
            "[tsf] input profiles skip ActivateLanguageProfile: zero profile guid for {}",
            guid_to_registry_string(&tip_info.clsid)
        ));
        return;
    }

    let activate_result =
        profiles.ActivateLanguageProfile(&tip_info.clsid, tip_info.lang_id, &tip_info.profile_guid);
    tsf_step(format!(
        "[tsf] input profiles ActivateLanguageProfile clsid={} lang=0x{:04X} profile={} -> {:?}",
        guid_to_registry_string(&tip_info.clsid),
        tip_info.lang_id,
        guid_to_registry_string(&tip_info.profile_guid),
        activate_result
    ));

    let mut active_lang = 0u16;
    let mut active_profile_guid = windows::core::GUID::zeroed();
    let active_profile = profiles.GetActiveLanguageProfile(
        &tip_info.clsid,
        &mut active_lang,
        &mut active_profile_guid,
    );
    tsf_step(format!(
        "[tsf] input profiles GetActiveLanguageProfile clsid={} -> {:?} lang=0x{:04X} profile={}",
        guid_to_registry_string(&tip_info.clsid),
        active_profile,
        active_lang,
        guid_to_registry_string(&active_profile_guid)
    ));
}

unsafe fn fill_tip_profile_from_input_profiles(tip_info: &mut TipInfo) {
    if tip_info.profile_guid == windows::core::GUID::zeroed() {
        let profiles: ITfInputProcessorProfiles =
            match CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER) {
                Ok(p) => p,
                Err(e) => {
                    tsf_step(format!(
                        "[tsf] input profiles unavailable for profile fill: {:?}",
                        e
                    ));
                    return;
                }
            };

        for lang in [tip_info.lang_id, 0x0411, 0x0409] {
            let Ok(enum_profiles) = profiles.EnumLanguageProfiles(lang) else {
                continue;
            };
            loop {
                let mut fetched = 0u32;
                let mut items = [TF_LANGUAGEPROFILE::default(); 1];
                if enum_profiles.Next(&mut items, &mut fetched).is_err() || fetched == 0 {
                    break;
                }

                let item = items[0];
                if item.clsid != tip_info.clsid || item.catid != GUID_TFCAT_TIP_KEYBOARD {
                    continue;
                }

                tip_info.lang_id = item.langid;
                tip_info.profile_guid = item.guidProfile;
                if let Ok(desc) = profiles.GetLanguageProfileDescription(
                    &tip_info.clsid,
                    item.langid,
                    &item.guidProfile,
                ) {
                    let desc = desc.to_string();
                    if !desc.trim().is_empty() {
                        tip_info.description = desc;
                    }
                }
                tsf_step(format!(
                    "[tsf] Filled TIP profile from ITfInputProcessorProfiles: clsid={} lang=0x{:04X} profile={} active={} profile_type={}",
                    guid_to_registry_string(&tip_info.clsid),
                    tip_info.lang_id,
                    guid_to_registry_string(&tip_info.profile_guid),
                    item.fActive.as_bool(),
                    TF_PROFILETYPE_INPUTPROCESSOR
                ));
                break;
            }

            if tip_info.profile_guid != windows::core::GUID::zeroed() {
                break;
            }
        }
    }
}

impl TsfRimeAdapter {
    fn build_key_state(&self, keycode: u32, modifier: u32) -> [u8; 256] {
        let mut key_state = [0u8; 256];
        let is_keyup = (modifier & (1 << 30)) != 0;
        let is_shift = (modifier & 1) != 0 || win_keymap::is_shifted_char(keycode);
        let is_ctrl = (modifier & 4) != 0;
        let is_alt = (modifier & 8) != 0;
        let vk = win_keymap::rime_to_vk(keycode);

        if is_shift {
            key_state[VK_SHIFT.0 as usize] = 0x80;
            key_state[VK_LSHIFT.0 as usize] = 0x80;
        }
        if is_ctrl {
            key_state[VK_CONTROL.0 as usize] = 0x80;
            key_state[VK_LCONTROL.0 as usize] = 0x80;
        }
        if is_alt {
            key_state[VK_MENU.0 as usize] = 0x80;
            key_state[VK_LMENU.0 as usize] = 0x80;
        }

        let idx = vk.0 as usize;
        if idx < key_state.len() && !is_keyup {
            key_state[idx] = 0x80;
        }

        key_state
    }

    fn install_key_state(&self, keycode: u32, modifier: u32) {
        let key_state = self.build_key_state(keycode, modifier);
        unsafe {
            let _ = SetKeyboardState(&key_state);
        }
    }

    fn pump_thread_messages(&self, reason: &str) {
        unsafe {
            let mut total = 0usize;
            loop {
                let mut msg = MSG::default();
                if !PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    break;
                }
                total += 1;
                let _ = TranslateMessage(&msg);
                let _ = DispatchMessageW(&msg);
                if total >= 256 {
                    tsf_step(format!(
                        "[tsf] pump_thread_messages reason={} hit cap={}",
                        reason, total
                    ));
                    break;
                }
            }
            tsf_step(format!(
                "[tsf] pump_thread_messages reason={} processed={}",
                reason, total
            ));
        }
    }

    fn settle_focus_async(&self, reason: &str) {
        self.pump_thread_messages(reason);
        std::thread::sleep(Duration::from_millis(15));
        self.pump_thread_messages(&format!("{}+sleep", reason));
    }

    /// Build a TSF backend from a resolved [`TipInfo`].
    pub unsafe fn new(mut tip_info: TipInfo) -> Result<Self, String> {
        tsf_step(format!(
            "[tsf] TsfRimeAdapter::new: tip clsid={} lang=0x{:04X} profile={} description={}",
            guid_to_registry_string(&tip_info.clsid),
            tip_info.lang_id,
            guid_to_registry_string(&tip_info.profile_guid),
            tip_info.description
        ));

        tsf_step("[tsf] CoInitializeEx(COINIT_APARTMENTTHREADED)");
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| format!("CoInitializeEx failed: {:?}", e))?;
        tsf_step("[tsf] CoInitializeEx ok");

        fill_tip_profile_from_input_profiles(&mut tip_info);

        tsf_step("[tsf] CoCreateInstance(CLSID_TF_ThreadMgr)");
        let inner_tm: ITfThreadMgr =
            CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| format!("CoCreateInstance(CLSID_TF_ThreadMgr): {:?}", e))?;
        tsf_step("[tsf] CoCreateInstance(CLSID_TF_ThreadMgr) ok");

        tsf_step("[tsf] QI(ITfKeystrokeMgr)");
        let inner_tm_ex = inner_tm
            .cast()
            .map_err(|e| format!("QI(ITfThreadMgrEx): {:?}", e))?;
        let inner_ks: ITfKeystrokeMgr = inner_tm
            .cast()
            .map_err(|e| format!("QI(ITfKeystrokeMgr): {:?}", e))?;
        let inner_source = inner_tm
            .cast()
            .map_err(|e| format!("QI(ITfSource): {:?}", e))?;
        let inner_source_single = inner_tm
            .cast()
            .map_err(|e| format!("QI(ITfSourceSingle): {:?}", e))?;
        let inner_compartment_mgr = inner_tm
            .cast()
            .map_err(|e| format!("QI(ITfCompartmentMgr): {:?}", e))?;
        let inner_lang_bar_item_mgr = acquire_lang_bar_item_mgr(&inner_tm);
        let inner_message_pump = inner_tm
            .cast()
            .map_err(|e| format!("QI(ITfMessagePump): {:?}", e))?;
        if inner_lang_bar_item_mgr.is_none() {
            tsf_step("[tsf] ITfLangBarItemMgr unavailable from Wine; using proxy fallback");
        }
        tsf_step("[tsf] QI(ITfKeystrokeMgr) ok");

        let state = TsfHostState::new(
            inner_tm.clone(),
            inner_tm_ex,
            inner_ks.clone(),
            inner_source,
            inner_source_single,
            inner_compartment_mgr,
            inner_lang_bar_item_mgr,
            inner_message_pump,
            tip_info.clsid,
            tip_info.lang_id,
            tip_info.profile_guid,
        );

        let wrap = ThreadMgrProxy::new(state.clone());
        let thread_mgr: ITfThreadMgr = wrap.into();
        tsf_step("[tsf] ThreadMgrProxy -> ITfThreadMgr ok");

        tsf_step("[tsf] ITfThreadMgr::Activate (TfClientId)");
        let client_id = thread_mgr
            .Activate()
            .map_err(|e| format!("ITfThreadMgr::Activate: {:?}", e))?;
        tsf_step(format!(
            "[tsf] ITfThreadMgr::Activate ok client_id={}",
            client_id
        ));

        tsf_step("[tsf] load_and_activate_tip ...");
        let loaded_tip = load_and_activate_tip(&tip_info, &thread_mgr, client_id)?;
        tsf_step("[tsf] load_and_activate_tip returned");

        tsf_step("[tsf] syncing input processor profile state ...");
        sync_input_processor_profile(&mut tip_info);
        tsf_step("[tsf] input processor profile sync done");

        tsf_step("[tsf] TsfRimeAdapter::new complete");
        Ok(Self {
            _com: CoInitGuard,
            state,
            thread_mgr,
            client_id,
            loaded_tip,
            sessions: HashMap::new(),
            next_id: 0,
        })
    }

    /// Resolve from CLI-style options and construct the adapter.
    pub unsafe fn from_options(
        tip_clsid: Option<&str>,
        tip_name: Option<&str>,
        tip_dll: Option<&str>,
    ) -> Result<Self, String> {
        tsf_step(format!(
            "[tsf] from_options: tip_clsid={:?} tip_name={:?} tip_dll={:?}",
            tip_clsid, tip_name, tip_dll
        ));
        let info = resolve_tip(tip_clsid, tip_name, tip_dll)?;
        tsf_step(format!(
            "[tsf] resolve_tip -> clsid={} lang=0x{:04X} profile={} dll={} description={}",
            guid_to_registry_string(&info.clsid),
            info.lang_id,
            guid_to_registry_string(&info.profile_guid),
            info.dll_path,
            info.description
        ));
        Self::new(info)
    }
}

impl Drop for TsfRimeAdapter {
    fn drop(&mut self) {
        if self.loaded_tip.activated {
            tsf_step("[tsf] TsfRimeAdapter::drop: ITfTextInputProcessor::Deactivate");
            let deact_result = unsafe { self.loaded_tip.tip.Deactivate() };
            tsf_step(format!("[tsf] Deactivate result: {:?}", deact_result));
        } else {
            tsf_step("[tsf] TsfRimeAdapter::drop: skip Deactivate (TIP never activated)");
        }
        tsf_step("[tsf] TsfRimeAdapter::drop done");
    }
}

// COM handles are not Send/Sync in Rust's model; the gRPC server uses this backend from one
// worker thread (see ChannelRimeBackend), matching Win32 single-threaded apartment usage.
unsafe impl Send for TsfRimeAdapter {}
unsafe impl Sync for TsfRimeAdapter {}

#[tonic::async_trait]
impl RimeBackend for TsfRimeAdapter {
    async fn open_session(&mut self) -> Option<usize> {
        tsf_step(format!(
            "[tsf] open_session: begin client_id={}",
            self.client_id
        ));
        unsafe {
            tsf_step("[tsf] open_session: CreateDocumentMgr");
            let doc_mgr = match self.thread_mgr.CreateDocumentMgr() {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!("CreateDocumentMgr: {:?}", e);
                    return None;
                }
            };
            tsf_step("[tsf] open_session: CreateDocumentMgr ok");

            let text_inner = TextStoreInner::new();
            let store = TextStoreAcp::with_inner(text_inner.clone());
            let store_iface: ITextStoreACP = store.into();
            let hwnd = text_inner.hwnd();
            let unk = match store_iface.cast::<windows::core::IUnknown>() {
                Ok(u) => u,
                Err(e) => {
                    tracing::error!("cast ITextStoreACP to IUnknown: {:?}", e);
                    return None;
                }
            };

            let mut ctx_opt = None;
            let mut cookie = 0u32;
            tsf_step("[tsf] open_session: CreateContext");
            if let Err(e) =
                doc_mgr.CreateContext(self.client_id, 0, &unk, &mut ctx_opt, &mut cookie)
            {
                tracing::error!("CreateContext: {:?}", e);
                return None;
            }
            tsf_step("[tsf] open_session: CreateContext ok");

            let ctx = match ctx_opt {
                Some(c) => c,
                None => {
                    tracing::error!("CreateContext returned null context");
                    return None;
                }
            };

            if let Ok(comp_mgr) = ctx.cast::<ITfCompartmentMgr>() {
                set_compartment_i4(&comp_mgr, self.client_id, &GUID_COMPARTMENT_EMPTYCONTEXT, 0);
                initialize_keyboard_compartments(&comp_mgr, self.client_id);
                tsf_step("[tsf] open_session: initialized context compartments");
                log_compartment_snapshot("context.after_init", &comp_mgr);
            }

            if let Ok(global_mgr) = self.thread_mgr.GetGlobalCompartment() {
                initialize_keyboard_compartments(&global_mgr, self.client_id);
                tsf_step("[tsf] open_session: initialized global TSF compartments");
                log_compartment_snapshot("global.after_init", &global_mgr);
            }

            tsf_step("[tsf] open_session: Push");
            if let Err(e) = doc_mgr.Push(&ctx) {
                tracing::error!("Push: {:?}", e);
                return None;
            }
            self.state.set_active_context(Some(ctx.clone()));
            self.state.notify_push_context(&ctx);
            tsf_step(format!(
                "[tsf] open_session: AssociateFocus hwnd={:?}",
                hwnd
            ));
            if let Err(e) = self.thread_mgr.AssociateFocus(hwnd, Some(&doc_mgr)) {
                tracing::warn!("AssociateFocus: {:?}", e);
            }
            tsf_step("[tsf] open_session: SetFocus");
            if let Err(e) = self.thread_mgr.SetFocus(Some(&doc_mgr)) {
                tracing::error!("SetFocus: {:?}", e);
                return None;
            }
            tsf_step("[tsf] open_session: SetFocus ok");
            if let Ok(current_focus) = self.thread_mgr.GetFocus() {
                tsf_step(format!(
                    "[tsf] open_session: current focus raw={:p} matches_doc_mgr={}",
                    current_focus.as_raw(),
                    current_focus.as_raw() == doc_mgr.as_raw()
                ));
            }
            if let Ok(ks) = self.thread_mgr.cast::<ITfKeystrokeMgr>() {
                tsf_step(format!(
                    "[tsf] open_session: GetForeground -> {:?}",
                    ks.GetForeground()
                ));
            }
            self.settle_focus_async("open_session_set_focus");

            self.next_id += 1;
            let id = self.next_id;

            self.sessions.insert(
                id,
                TsfSession {
                    doc_mgr,
                    context: ctx,
                    text_store: store_iface,
                    text: text_inner,
                    pending_commit: None,
                    last_preedit: String::new(),
                },
            );

            tsf_step(format!("[tsf] open_session: done session_id={}", id));
            Some(id)
        }
    }

    async fn destroy_session(&mut self, session_id: usize) {
        if let Some(sess) = self.sessions.remove(&session_id) {
            self.state.set_active_context(None);
            unsafe {
                let hwnd = sess.text.hwnd();
                let _ = self.thread_mgr.SetFocus(None);
                if !hwnd.is_invalid() {
                    let _ = self.thread_mgr.AssociateFocus(hwnd, None);
                }
            }
            self.state.notify_pop_context(&sess.context);
            unsafe {
                let _ = sess.doc_mgr.Pop(0);
            }
            self.settle_focus_async("destroy_session_clear_focus");
        }
    }

    async fn process_key(&mut self, session_id: usize, key: &KeyEvent) -> bool {
        tsf_step(format!(
            "[tsf] process_key: session_id={} keycode={} modifier=0x{:X}",
            session_id, key.keycode, key.modifier
        ));
        let Some(sess) = self.sessions.get(&session_id) else {
            tsf_step(format!(
                "[tsf] process_key: unknown session {}, ignore",
                session_id
            ));
            return false;
        };
        let doc_mgr = sess.doc_mgr.clone();
        let text = sess.text.clone();
        let pre_key_preedit = text.snapshot_utf8();
        let pre_key_highlighted =
            unsafe { tsf_candidates::highlighted_candidate_text(&self.thread_mgr, Some(&doc_mgr)) };

        self.settle_focus_async("process_key_pre");

        let rime_mod = key.modifier;
        let is_keyup = (rime_mod & (1 << 30)) != 0;
        let is_alt = (rime_mod & 8) != 0;
        let is_ctrl = (rime_mod & 4) != 0;
        if (is_ctrl || is_alt) && !(is_ctrl && is_alt) {
            tsf_step("[tsf] process_key: plain Ctrl/Alt chord ignored");
            return false;
        }

        let _is_shift = (rime_mod & 1) != 0 || win_keymap::is_shifted_char(key.keycode);
        let vk = win_keymap::rime_to_vk(key.keycode);
        let lparam = win_keymap::make_l_key_data(vk, is_keyup, is_alt);
        let wparam = windows::Win32::Foundation::WPARAM(vk.0 as usize);
        self.install_key_state(key.keycode, key.modifier);

        let ks: ITfKeystrokeMgr = match self.thread_mgr.cast() {
            Ok(k) => k,
            Err(e) => {
                tsf_step(format!(
                    "[tsf] process_key: QI(ITfKeystrokeMgr) failed: {:?}",
                    e
                ));
                return false;
            }
        };

        match unsafe { self.thread_mgr.GetFocus() } {
            Ok(current_focus) => tsf_step(format!(
                "[tsf] process_key: current focus raw={:p} session_doc_mgr={:p} matches={}",
                current_focus.as_raw(),
                doc_mgr.as_raw(),
                current_focus.as_raw() == doc_mgr.as_raw()
            )),
            Err(e) => tsf_step(format!("[tsf] process_key: GetFocus failed: {:?}", e)),
        }
        tsf_step(format!(
            "[tsf] process_key: GetForeground -> {:?}",
            unsafe { ks.GetForeground() }
        ));
        if let Ok(comp_mgr) = sess.context.cast::<ITfCompartmentMgr>() {
            unsafe {
                log_compartment_snapshot("process_key.context_pre", &comp_mgr);
            }
        }
        if let Ok(global_mgr) = unsafe { self.thread_mgr.GetGlobalCompartment() } {
            unsafe {
                log_compartment_snapshot("process_key.global_pre", &global_mgr);
            }
        }

        let lparam = windows::Win32::Foundation::LPARAM(lparam as isize);
        let tested = unsafe {
            if is_keyup {
                ks.TestKeyUp(wparam, lparam)
            } else {
                ks.TestKeyDown(wparam, lparam)
            }
        }
        .map(|b| b.as_bool())
        .unwrap_or(false);

        tsf_step(format!(
            "[tsf] process_key: vk=0x{:X} test_{} -> {}",
            vk.0,
            if is_keyup { "up" } else { "down" },
            tested
        ));

        if !tested {
            return false;
        }

        let handled = unsafe {
            if is_keyup {
                ks.KeyUp(wparam, lparam)
            } else {
                ks.KeyDown(wparam, lparam)
            }
        }
        .map(|b| b.as_bool())
        .unwrap_or(false);

        tsf_step(format!(
            "[tsf] process_key: key_{} -> {} text='{}'",
            if is_keyup { "up" } else { "down" },
            handled,
            text.snapshot_utf8()
        ));

        self.settle_focus_async("process_key_post");
        let post_key_preedit = text.snapshot_utf8();
        if handled && !is_keyup {
            if !pre_key_preedit.is_empty() && post_key_preedit.is_empty() {
                if let Some(commit_text) = pre_key_highlighted
                    .filter(|s| !s.is_empty())
                    .or_else(|| Some(pre_key_preedit.clone()).filter(|s| !s.is_empty()))
                {
                    if let Some(sess_mut) = self.sessions.get_mut(&session_id) {
                        sess_mut.pending_commit = Some(commit_text.clone());
                        tsf_step(format!(
                            "[tsf] process_key: inferred commit='{}' from transition",
                            commit_text
                        ));
                    }
                }
            }
        }
        if let Some(sess_mut) = self.sessions.get_mut(&session_id) {
            sess_mut.last_preedit = post_key_preedit;
        }

        handled
    }

    async fn get_context(&mut self, session_id: usize) -> RimeContextProto {
        self.settle_focus_async("get_context_pre");
        let thread_mgr = self.thread_mgr.clone();
        if let Some(sess) = self.sessions.get(&session_id) {
            let s = sess.text.snapshot_utf8();
            let menu = unsafe {
                tsf_candidates::menu_from_ui_element_mgr(&thread_mgr, Some(&sess.doc_mgr))
            };
            let mut commit_text_preview = sess.pending_commit.clone().unwrap_or_default();
            if sess.pending_commit.is_none() && !sess.last_preedit.is_empty() && s.is_empty() {
                if let Some(commit_text) = unsafe {
                    tsf_candidates::highlighted_candidate_text(&thread_mgr, Some(&sess.doc_mgr))
                } {
                    if !commit_text.is_empty() {
                        commit_text_preview = commit_text.clone();
                        if let Some(sess_mut) = self.sessions.get_mut(&session_id) {
                            sess_mut.pending_commit = Some(commit_text);
                        }
                    }
                }
            }
            if let Some(sess_mut) = self.sessions.get_mut(&session_id) {
                sess_mut.last_preedit = s.clone();
            }
            tsf_step(format!(
                "[tsf] get_context: session_id={} preedit='{}' menu_candidates={}",
                session_id,
                s,
                menu.as_ref().map(|m| m.num_candidates).unwrap_or(0)
            ));
            return RimeContextProto {
                composition: if s.is_empty() {
                    None
                } else {
                    Some(crate::proto::rime_service_v2::CompositionProto {
                        length: s.len() as i32,
                        cursor_pos: s.len() as i32,
                        sel_start: 0,
                        sel_end: s.len() as i32,
                        preedit: s,
                    })
                },
                menu,
                commit_text_preview,
            };
        }
        RimeContextProto::default()
    }

    async fn get_commit(&mut self, session_id: usize) -> Option<String> {
        self.sessions
            .get_mut(&session_id)
            .and_then(|sess| sess.pending_commit.take())
    }

    async fn select_candidate(&mut self, session_id: usize, index: usize) -> bool {
        let Some(sess) = self.sessions.get(&session_id) else {
            return false;
        };
        let doc_mgr = sess.doc_mgr.clone();
        let text = sess.text.clone();
        let commit_hint = unsafe {
            tsf_candidates::candidate_text_at_index(&self.thread_mgr, Some(&doc_mgr), index as u32)
        };

        self.settle_focus_async("select_candidate_pre");
        let selected = unsafe {
            tsf_candidates::select_candidate_by_index(
                &self.thread_mgr,
                Some(&doc_mgr),
                index as u32,
            )
        };
        self.settle_focus_async("select_candidate_post");
        if !selected {
            return false;
        }

        let post_preedit = text.snapshot_utf8();
        if let Some(sess_mut) = self.sessions.get_mut(&session_id) {
            sess_mut.last_preedit = post_preedit.clone();
            if let Some(commit_text) = commit_hint.filter(|s| !s.is_empty()) {
                sess_mut.pending_commit = Some(commit_text.clone());
                tsf_step(format!(
                    "[tsf] select_candidate: commit_hint='{}' index={} post_preedit='{}'",
                    commit_text, index, post_preedit
                ));
            }
        }
        true
    }
}
