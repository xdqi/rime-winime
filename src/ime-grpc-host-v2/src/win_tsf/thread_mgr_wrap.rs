//! Thread manager proxy that gives TIPs a single COM identity while filling Wine's TSF gaps.

use std::sync::{Arc, Mutex};

use windows::core::{implement, Interface, BSTR, GUID, PCWSTR};
use windows::Win32::Foundation::{
    BOOL, E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL, HWND, LPARAM, RECT, S_FALSE, WPARAM,
};
use windows::Win32::System::Com::{CoCreateInstance, IEnumGUID, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
use windows::Win32::UI::TextServices::{
    CLSID_TF_LangBarMgr, IEnumTfDocumentMgrs, IEnumTfFunctionProviders, IEnumTfLangBarItems,
    IEnumTfUIElements, ITfActiveLanguageProfileNotifySink, ITfClientId, ITfClientId_Impl,
    ITfCompartment, ITfCompartmentMgr, ITfCompartmentMgr_Impl, ITfContext, ITfDocumentMgr,
    ITfFunctionProvider, ITfInputProcessorProfileActivationSink, ITfKeyEventSink, ITfKeystrokeMgr,
    ITfKeystrokeMgr_Impl, ITfLangBarItem, ITfLangBarItemMgr, ITfLangBarItemMgr_Impl,
    ITfLangBarItemSink, ITfLangBarMgr, ITfMessagePump, ITfMessagePump_Impl, ITfSource,
    ITfSourceSingle, ITfSourceSingle_Impl, ITfSource_Impl, ITfThreadFocusSink, ITfThreadMgr,
    ITfThreadMgrEventSink, ITfThreadMgrEx, ITfThreadMgrEx_Impl, ITfThreadMgr_Impl, ITfUIElement,
    ITfUIElementMgr, ITfUIElementMgr_Impl, ITfUIElementSink, GUID_TFCAT_TIP_KEYBOARD,
    TF_IPSINK_FLAG_ACTIVE, TF_LANGBARITEMINFO, TF_PRESERVEDKEY, TF_PROFILETYPE_INPUTPROCESSOR,
};

use super::diag::{tsf_step, tsf_warn};

fn describe_source_iid(iid: &GUID) -> &'static str {
    if *iid == ITfThreadMgrEventSink::IID {
        "ITfThreadMgrEventSink"
    } else if *iid == ITfThreadFocusSink::IID {
        "ITfThreadFocusSink"
    } else if *iid == ITfActiveLanguageProfileNotifySink::IID {
        "ITfActiveLanguageProfileNotifySink"
    } else if *iid == ITfInputProcessorProfileActivationSink::IID {
        "ITfInputProcessorProfileActivationSink"
    } else if *iid == ITfUIElementSink::IID {
        "ITfUIElementSink"
    } else {
        "unknown"
    }
}

pub unsafe fn acquire_lang_bar_item_mgr(inner_tm: &ITfThreadMgr) -> Option<ITfLangBarItemMgr> {
    if let Ok(item_mgr) = inner_tm.cast::<ITfLangBarItemMgr>() {
        tsf_step("[tsf] acquired ITfLangBarItemMgr via direct QI on ITfThreadMgr");
        return Some(item_mgr);
    }

    let lang_bar_mgr: ITfLangBarMgr =
        CoCreateInstance(&CLSID_TF_LangBarMgr, None, CLSCTX_INPROC_SERVER).ok()?;
    let mut item_mgr = None;
    let mut actual_thread_id = 0u32;
    lang_bar_mgr
        .GetThreadLangBarItemMgr(GetCurrentThreadId(), &mut item_mgr, &mut actual_thread_id)
        .ok()?;
    if let Some(ref mgr) = item_mgr {
        tsf_step(format!(
            "[tsf] acquired ITfLangBarItemMgr via ITfLangBarMgr thread={} actual_thread={}",
            GetCurrentThreadId(),
            actual_thread_id
        ));
        Some(mgr.clone())
    } else {
        None
    }
}

#[derive(Clone)]
struct FunctionProviderRegistration {
    tid: u32,
    provider: ITfFunctionProvider,
    provider_type: Option<GUID>,
}

#[implement(IEnumTfFunctionProviders)]
struct FunctionProviderEnum {
    providers: Mutex<Vec<ITfFunctionProvider>>,
    cursor: Mutex<usize>,
}

impl FunctionProviderEnum {
    fn new(providers: Vec<ITfFunctionProvider>) -> Self {
        Self {
            providers: Mutex::new(providers),
            cursor: Mutex::new(0),
        }
    }
}

impl windows::Win32::UI::TextServices::IEnumTfFunctionProviders_Impl for FunctionProviderEnum {
    fn Clone(&self) -> windows::core::Result<IEnumTfFunctionProviders> {
        let providers = self.providers.lock().unwrap().clone();
        let cursor = *self.cursor.lock().unwrap();
        Ok(Self {
            providers: Mutex::new(providers),
            cursor: Mutex::new(cursor),
        }
        .into())
    }

    fn Next(
        &self,
        ulcount: u32,
        ppcmdobj: *mut Option<ITfFunctionProvider>,
        pcfetch: *mut u32,
    ) -> windows::core::Result<()> {
        if ppcmdobj.is_null() {
            return Err(E_INVALIDARG.into());
        }

        let providers = self.providers.lock().unwrap();
        let mut cursor = self.cursor.lock().unwrap();
        let remaining = providers.len().saturating_sub(*cursor);
        let fetched = remaining.min(ulcount as usize);

        unsafe {
            for i in 0..fetched {
                ppcmdobj.add(i).write(Some(providers[*cursor + i].clone()));
            }
            for i in fetched..ulcount as usize {
                ppcmdobj.add(i).write(None);
            }
            if !pcfetch.is_null() {
                *pcfetch = fetched as u32;
            }
        }

        *cursor += fetched;
        if fetched == ulcount as usize {
            Ok(())
        } else {
            Err(S_FALSE.into())
        }
    }

    fn Reset(&self) -> windows::core::Result<()> {
        *self.cursor.lock().unwrap() = 0;
        Ok(())
    }

    fn Skip(&self, ulcount: u32) -> windows::core::Result<()> {
        let len = self.providers.lock().unwrap().len();
        let mut cursor = self.cursor.lock().unwrap();
        *cursor = (*cursor).saturating_add(ulcount as usize).min(len);
        if *cursor < len {
            Ok(())
        } else {
            Err(S_FALSE.into())
        }
    }
}

impl windows::Win32::UI::TextServices::IEnumTfFunctionProviders_Impl for FunctionProviderEnum_Impl {
    fn Clone(&self) -> windows::core::Result<IEnumTfFunctionProviders> {
        windows::Win32::UI::TextServices::IEnumTfFunctionProviders_Impl::Clone(&self.this)
    }

    fn Next(
        &self,
        ulcount: u32,
        ppcmdobj: *mut Option<ITfFunctionProvider>,
        pcfetch: *mut u32,
    ) -> windows::core::Result<()> {
        windows::Win32::UI::TextServices::IEnumTfFunctionProviders_Impl::Next(
            &self.this, ulcount, ppcmdobj, pcfetch,
        )
    }

    fn Reset(&self) -> windows::core::Result<()> {
        windows::Win32::UI::TextServices::IEnumTfFunctionProviders_Impl::Reset(&self.this)
    }

    fn Skip(&self, ulcount: u32) -> windows::core::Result<()> {
        windows::Win32::UI::TextServices::IEnumTfFunctionProviders_Impl::Skip(&self.this, ulcount)
    }
}

#[derive(Clone)]
struct SourceSingleRegistration {
    tid: u32,
    riid: GUID,
}

#[derive(Clone)]
struct LocalLangBarItemRegistration {
    item: ITfLangBarItem,
    guid: Option<GUID>,
}

#[implement(IEnumTfLangBarItems)]
struct LangBarItemEnum {
    items: Mutex<Vec<ITfLangBarItem>>,
    cursor: Mutex<usize>,
}

impl LangBarItemEnum {
    fn new(items: Vec<ITfLangBarItem>) -> Self {
        Self {
            items: Mutex::new(items),
            cursor: Mutex::new(0),
        }
    }
}

impl windows::Win32::UI::TextServices::IEnumTfLangBarItems_Impl for LangBarItemEnum {
    fn Clone(&self) -> windows::core::Result<IEnumTfLangBarItems> {
        let items = self.items.lock().unwrap().clone();
        let cursor = *self.cursor.lock().unwrap();
        Ok(Self {
            items: Mutex::new(items),
            cursor: Mutex::new(cursor),
        }
        .into())
    }

    fn Next(
        &self,
        ulcount: u32,
        ppitem: *mut Option<ITfLangBarItem>,
        pcfetched: *mut u32,
    ) -> windows::core::Result<()> {
        if ppitem.is_null() {
            return Err(E_INVALIDARG.into());
        }

        let items = self.items.lock().unwrap();
        let mut cursor = self.cursor.lock().unwrap();
        let remaining = items.len().saturating_sub(*cursor);
        let fetched = remaining.min(ulcount as usize);

        unsafe {
            for i in 0..fetched {
                ppitem.add(i).write(Some(items[*cursor + i].clone()));
            }
            for i in fetched..ulcount as usize {
                ppitem.add(i).write(None);
            }
            if !pcfetched.is_null() {
                *pcfetched = fetched as u32;
            }
        }

        *cursor += fetched;
        if fetched == ulcount as usize {
            Ok(())
        } else {
            Err(S_FALSE.into())
        }
    }

    fn Reset(&self) -> windows::core::Result<()> {
        *self.cursor.lock().unwrap() = 0;
        Ok(())
    }

    fn Skip(&self, ulcount: u32) -> windows::core::Result<()> {
        let len = self.items.lock().unwrap().len();
        let mut cursor = self.cursor.lock().unwrap();
        *cursor = (*cursor).saturating_add(ulcount as usize).min(len);
        if *cursor < len {
            Ok(())
        } else {
            Err(S_FALSE.into())
        }
    }
}

impl windows::Win32::UI::TextServices::IEnumTfLangBarItems_Impl for LangBarItemEnum_Impl {
    fn Clone(&self) -> windows::core::Result<IEnumTfLangBarItems> {
        windows::Win32::UI::TextServices::IEnumTfLangBarItems_Impl::Clone(&self.this)
    }

    fn Next(
        &self,
        ulcount: u32,
        ppitem: *mut Option<ITfLangBarItem>,
        pcfetched: *mut u32,
    ) -> windows::core::Result<()> {
        windows::Win32::UI::TextServices::IEnumTfLangBarItems_Impl::Next(
            &self.this, ulcount, ppitem, pcfetched,
        )
    }

    fn Reset(&self) -> windows::core::Result<()> {
        windows::Win32::UI::TextServices::IEnumTfLangBarItems_Impl::Reset(&self.this)
    }

    fn Skip(&self, ulcount: u32) -> windows::core::Result<()> {
        windows::Win32::UI::TextServices::IEnumTfLangBarItems_Impl::Skip(&self.this, ulcount)
    }
}

/// Shared TSF state across the adapter and thread manager proxy.
pub struct TsfHostState {
    pub inner_tm: ITfThreadMgr,
    pub inner_tm_ex: ITfThreadMgrEx,
    pub inner_ks: ITfKeystrokeMgr,
    pub inner_source: ITfSource,
    pub inner_source_single: ITfSourceSingle,
    pub inner_compartment_mgr: ITfCompartmentMgr,
    pub inner_lang_bar_item_mgr: Option<ITfLangBarItemMgr>,
    pub inner_message_pump: ITfMessagePump,
    pub tip_clsid: GUID,
    pub lang_id: u16,
    pub profile_guid: GUID,
    pub client_id: Mutex<Option<u32>>,
    pub sink: Mutex<Option<(u32, ITfKeyEventSink)>>,
    pub foreground_tip: Mutex<Option<GUID>>,
    pub proxy_key_sink_only: Mutex<bool>,
    pub active_context: Mutex<Option<ITfContext>>,
    pub thread_mgr_event_sink: Mutex<Option<ITfThreadMgrEventSink>>,
    pub thread_focus_sink: Mutex<Option<ITfThreadFocusSink>>,
    pub active_lang_profile_sink: Mutex<Option<ITfActiveLanguageProfileNotifySink>>,
    pub profile_activation_sink: Mutex<Option<ITfInputProcessorProfileActivationSink>>,
    function_providers: Mutex<Vec<FunctionProviderRegistration>>,
    source_single_sinks: Mutex<Vec<SourceSingleRegistration>>,
    local_lang_bar_items: Mutex<Vec<LocalLangBarItemRegistration>>,
    pub ui_element_sink: Mutex<Option<ITfUIElementSink>>,
    pub ui_elements: Mutex<Vec<(u32, ITfUIElement)>>,
    pub next_ui_element_id: Mutex<u32>,
}

impl TsfHostState {
    pub fn new(
        inner_tm: ITfThreadMgr,
        inner_tm_ex: ITfThreadMgrEx,
        inner_ks: ITfKeystrokeMgr,
        inner_source: ITfSource,
        inner_source_single: ITfSourceSingle,
        inner_compartment_mgr: ITfCompartmentMgr,
        inner_lang_bar_item_mgr: Option<ITfLangBarItemMgr>,
        inner_message_pump: ITfMessagePump,
        tip_clsid: GUID,
        lang_id: u16,
        profile_guid: GUID,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner_tm,
            inner_tm_ex,
            inner_ks,
            inner_source,
            inner_source_single,
            inner_compartment_mgr,
            inner_lang_bar_item_mgr,
            inner_message_pump,
            tip_clsid,
            lang_id,
            profile_guid,
            client_id: Mutex::new(None),
            sink: Mutex::new(None),
            foreground_tip: Mutex::new(None),
            proxy_key_sink_only: Mutex::new(false),
            active_context: Mutex::new(None),
            thread_mgr_event_sink: Mutex::new(None),
            thread_focus_sink: Mutex::new(None),
            active_lang_profile_sink: Mutex::new(None),
            profile_activation_sink: Mutex::new(None),
            function_providers: Mutex::new(Vec::new()),
            source_single_sinks: Mutex::new(Vec::new()),
            local_lang_bar_items: Mutex::new(Vec::new()),
            ui_element_sink: Mutex::new(None),
            ui_elements: Mutex::new(Vec::new()),
            next_ui_element_id: Mutex::new(1),
        })
    }

    pub fn set_active_context(&self, ctx: Option<ITfContext>) {
        *self.active_context.lock().unwrap() = ctx;
    }

    pub fn note_client_id(&self, client_id: u32) {
        *self.client_id.lock().unwrap() = Some(client_id);
    }

    fn notify_init_document_mgr(&self, doc_mgr: &ITfDocumentMgr) {
        if let Some(sink) = self.thread_mgr_event_sink.lock().unwrap().clone() {
            unsafe {
                let _ = sink.OnInitDocumentMgr(Some(doc_mgr));
            }
        }
    }

    fn notify_set_focus(&self, focus: Option<&ITfDocumentMgr>, prev: Option<&ITfDocumentMgr>) {
        if let Some(sink) = self.thread_mgr_event_sink.lock().unwrap().clone() {
            unsafe {
                let _ = sink.OnSetFocus(focus, prev);
            }
        }
    }

    fn notify_key_sink_focus_local(&self, focused: bool, reason: &str) {
        if let Some((tid, sink)) = self.sink.lock().unwrap().clone() {
            unsafe {
                let _ = sink.OnSetFocus(focused);
            }
            tsf_step(format!(
                "[tsf] local ITfKeyEventSink::OnSetFocus focused={} tid={} reason={}",
                focused, tid, reason
            ));
        } else {
            tsf_step(format!(
                "[tsf] local ITfKeyEventSink::OnSetFocus skipped focused={} reason={} (no sink)",
                focused, reason
            ));
        }
    }

    fn notify_profile_activation_local(&self, active: bool) {
        if let Some(sink) = self.active_lang_profile_sink.lock().unwrap().clone() {
            unsafe {
                let _ = sink.OnActivated(&self.tip_clsid, &self.profile_guid, active);
            }
            tsf_step(format!(
                "[tsf] local ITfActiveLanguageProfileNotifySink::OnActivated active={}",
                active
            ));
        }
        if let Some(sink) = self.profile_activation_sink.lock().unwrap().clone() {
            unsafe {
                let _ = sink.OnActivated(
                    TF_PROFILETYPE_INPUTPROCESSOR,
                    self.lang_id,
                    &self.tip_clsid,
                    &GUID_TFCAT_TIP_KEYBOARD,
                    &self.profile_guid,
                    HKL(std::ptr::null_mut()),
                    if active { TF_IPSINK_FLAG_ACTIVE } else { 0 },
                );
            }
            tsf_step(format!(
                "[tsf] local ITfInputProcessorProfileActivationSink::OnActivated active={}",
                active
            ));
        }
    }

    fn notify_thread_focus_local(&self, focused: bool) {
        if let Some(sink) = self.thread_focus_sink.lock().unwrap().clone() {
            unsafe {
                let _ = if focused {
                    sink.OnSetThreadFocus()
                } else {
                    sink.OnKillThreadFocus()
                };
            }
            tsf_step(format!(
                "[tsf] local ITfThreadFocusSink::{}",
                if focused {
                    "OnSetThreadFocus"
                } else {
                    "OnKillThreadFocus"
                }
            ));
        }
    }

    pub fn notify_push_context(&self, ctx: &ITfContext) {
        if let Some(sink) = self.thread_mgr_event_sink.lock().unwrap().clone() {
            unsafe {
                let _ = sink.OnPushContext(Some(ctx));
            }
        }
    }

    pub fn notify_pop_context(&self, ctx: &ITfContext) {
        if let Some(sink) = self.thread_mgr_event_sink.lock().unwrap().clone() {
            unsafe {
                let _ = sink.OnPopContext(Some(ctx));
            }
        }
    }

    fn register_function_provider(&self, tid: u32, provider: ITfFunctionProvider) {
        let provider_type = unsafe { provider.GetType().ok() };
        tsf_step(format!(
            "[tsf] ITfSourceSingle::AdviseSingleSink ITfFunctionProvider tid={} type={:?}",
            tid, provider_type
        ));
        let mut providers = self.function_providers.lock().unwrap();
        if let Some(existing) = providers
            .iter_mut()
            .find(|p| p.tid == tid && p.provider.as_raw() == provider.as_raw())
        {
            existing.provider_type = provider_type;
            return;
        }
        providers.push(FunctionProviderRegistration {
            tid,
            provider,
            provider_type,
        });
    }

    fn unregister_function_provider(&self, tid: u32, riid: &GUID) {
        let mut providers = self.function_providers.lock().unwrap();
        if *riid == ITfFunctionProvider::IID {
            providers.retain(|p| p.tid != tid);
        }
    }

    fn find_function_provider(&self, clsid: *const GUID) -> Option<ITfFunctionProvider> {
        let want = unsafe { clsid.as_ref().copied() };
        let providers = self.function_providers.lock().unwrap();
        if let Some(want) = want {
            if let Some(found) = providers.iter().find(|p| p.provider_type == Some(want)) {
                return Some(found.provider.clone());
            }
        }
        providers.first().map(|p| p.provider.clone())
    }

    fn function_providers_snapshot(&self) -> Vec<ITfFunctionProvider> {
        self.function_providers
            .lock()
            .unwrap()
            .iter()
            .map(|entry| entry.provider.clone())
            .collect()
    }

    fn register_source_single_sink(&self, tid: u32, riid: GUID, _sink: windows::core::IUnknown) {
        self.source_single_sinks
            .lock()
            .unwrap()
            .retain(|entry| !(entry.tid == tid && entry.riid == riid));
        self.source_single_sinks
            .lock()
            .unwrap()
            .push(SourceSingleRegistration { tid, riid });
    }

    fn unregister_source_single_sink(&self, tid: u32, riid: &GUID) {
        self.source_single_sinks
            .lock()
            .unwrap()
            .retain(|entry| !(entry.tid == tid && entry.riid == *riid));
    }

    fn add_local_lang_bar_item(&self, item: &ITfLangBarItem) {
        let guid = unsafe {
            let mut info = std::mem::zeroed::<TF_LANGBARITEMINFO>();
            if item.GetInfo(&mut info).is_ok() {
                Some(info.guidItem)
            } else {
                None
            }
        };
        tsf_step(format!(
            "[tsf] ITfLangBarItemMgr::AddItem local guid={:?} raw={:p}",
            guid,
            item.as_raw()
        ));
        let mut items = self.local_lang_bar_items.lock().unwrap();
        if items
            .iter()
            .any(|entry| entry.item.as_raw() == item.as_raw())
        {
            return;
        }
        items.push(LocalLangBarItemRegistration {
            item: item.clone(),
            guid,
        });
    }

    fn remove_local_lang_bar_item(&self, item: &ITfLangBarItem) {
        tsf_step(format!(
            "[tsf] ITfLangBarItemMgr::RemoveItem local raw={:p}",
            item.as_raw()
        ));
        self.local_lang_bar_items
            .lock()
            .unwrap()
            .retain(|entry| entry.item.as_raw() != item.as_raw());
    }

    fn local_lang_bar_items_snapshot(&self) -> Vec<ITfLangBarItem> {
        self.local_lang_bar_items
            .lock()
            .unwrap()
            .iter()
            .map(|entry| entry.item.clone())
            .collect()
    }

    fn get_local_lang_bar_item(&self, guid: *const GUID) -> Option<ITfLangBarItem> {
        let want = unsafe { guid.as_ref().copied() };
        let items = self.local_lang_bar_items.lock().unwrap();
        let hit = items
            .iter()
            .find(|entry| want.is_some() && entry.guid == want);
        if let Some(hit) = hit {
            tsf_step(format!(
                "[tsf] ITfLangBarItemMgr::GetItem local-hit guid={:?}",
                want
            ));
            Some(hit.item.clone())
        } else {
            tsf_warn(format!(
                "[tsf] ITfLangBarItemMgr::GetItem local-miss guid={:?}",
                want
            ));
            None
        }
    }

    fn ui_element_begin(
        &self,
        element: &ITfUIElement,
        pbshow: *mut BOOL,
        pdwuielementid: *mut u32,
    ) {
        let mut next_id = self.next_ui_element_id.lock().unwrap();
        let id = *next_id;
        *next_id = next_id.saturating_add(1);
        self.ui_elements.lock().unwrap().push((id, element.clone()));
        unsafe {
            if !pdwuielementid.is_null() {
                *pdwuielementid = id;
            }
        }

        let sink = self.ui_element_sink.lock().unwrap().clone();
        if let Some(sink) = sink {
            unsafe {
                let _ = sink.BeginUIElement(id, pbshow);
            }
        } else {
            unsafe {
                if !pbshow.is_null() {
                    *pbshow = true.into();
                }
            }
        }
    }

    fn ui_element_update(&self, id: u32) {
        if let Some(sink) = self.ui_element_sink.lock().unwrap().clone() {
            unsafe {
                let _ = sink.UpdateUIElement(id);
            }
        }
    }

    fn ui_element_end(&self, id: u32) {
        self.ui_elements
            .lock()
            .unwrap()
            .retain(|(cur, _)| *cur != id);
        if let Some(sink) = self.ui_element_sink.lock().unwrap().clone() {
            unsafe {
                let _ = sink.EndUIElement(id);
            }
        }
    }

    fn ui_element_get(&self, id: u32) -> Option<ITfUIElement> {
        self.ui_elements
            .lock()
            .unwrap()
            .iter()
            .find(|(cur, _)| *cur == id)
            .map(|(_, element)| element.clone())
    }

    fn ui_element_snapshot(&self) -> Vec<ITfUIElement> {
        self.ui_elements
            .lock()
            .unwrap()
            .iter()
            .map(|(_, element)| element.clone())
            .collect()
    }
}

#[implement(IEnumTfUIElements)]
struct UiElementEnum {
    elements: Mutex<Vec<ITfUIElement>>,
    cursor: Mutex<usize>,
}

impl UiElementEnum {
    fn new(elements: Vec<ITfUIElement>) -> Self {
        Self {
            elements: Mutex::new(elements),
            cursor: Mutex::new(0),
        }
    }
}

impl windows::Win32::UI::TextServices::IEnumTfUIElements_Impl for UiElementEnum {
    fn Clone(&self) -> windows::core::Result<IEnumTfUIElements> {
        let elements = self.elements.lock().unwrap().clone();
        let cursor = *self.cursor.lock().unwrap();
        let clone = Self {
            elements: Mutex::new(elements),
            cursor: Mutex::new(cursor),
        };
        Ok(clone.into())
    }

    fn Next(
        &self,
        ulcount: u32,
        ppelement: *mut Option<ITfUIElement>,
        pcfetched: *mut u32,
    ) -> windows::core::Result<()> {
        if ppelement.is_null() {
            return Err(E_INVALIDARG.into());
        }

        let elements = self.elements.lock().unwrap();
        let mut cursor = self.cursor.lock().unwrap();
        let remaining = elements.len().saturating_sub(*cursor);
        let fetched = remaining.min(ulcount as usize);

        unsafe {
            for i in 0..fetched {
                ppelement.add(i).write(Some(elements[*cursor + i].clone()));
            }
            for i in fetched..(ulcount as usize) {
                ppelement.add(i).write(None);
            }
            if !pcfetched.is_null() {
                *pcfetched = fetched as u32;
            }
        }

        *cursor += fetched;
        Ok(())
    }

    fn Reset(&self) -> windows::core::Result<()> {
        *self.cursor.lock().unwrap() = 0;
        Ok(())
    }

    fn Skip(&self, ulcount: u32) -> windows::core::Result<()> {
        let len = self.elements.lock().unwrap().len();
        let mut cursor = self.cursor.lock().unwrap();
        *cursor = (*cursor).saturating_add(ulcount as usize).min(len);
        Ok(())
    }
}

impl windows::Win32::UI::TextServices::IEnumTfUIElements_Impl for UiElementEnum_Impl {
    fn Clone(&self) -> windows::core::Result<IEnumTfUIElements> {
        windows::Win32::UI::TextServices::IEnumTfUIElements_Impl::Clone(&self.this)
    }
    fn Next(
        &self,
        ulcount: u32,
        ppelement: *mut Option<ITfUIElement>,
        pcfetched: *mut u32,
    ) -> windows::core::Result<()> {
        windows::Win32::UI::TextServices::IEnumTfUIElements_Impl::Next(
            &self.this, ulcount, ppelement, pcfetched,
        )
    }
    fn Reset(&self) -> windows::core::Result<()> {
        windows::Win32::UI::TextServices::IEnumTfUIElements_Impl::Reset(&self.this)
    }
    fn Skip(&self, ulcount: u32) -> windows::core::Result<()> {
        windows::Win32::UI::TextServices::IEnumTfUIElements_Impl::Skip(&self.this, ulcount)
    }
}

#[implement(
    ITfThreadMgr,
    ITfThreadMgrEx,
    ITfKeystrokeMgr,
    ITfSource,
    ITfSourceSingle,
    ITfCompartmentMgr,
    ITfLangBarItemMgr,
    ITfMessagePump,
    ITfClientId,
    ITfUIElementMgr
)]
pub struct ThreadMgrProxy {
    state: Arc<TsfHostState>,
}

impl ThreadMgrProxy {
    pub fn new(state: Arc<TsfHostState>) -> Self {
        Self { state }
    }
}

impl ITfThreadMgr_Impl for ThreadMgrProxy {
    fn Activate(&self) -> windows::core::Result<u32> {
        let client_id = unsafe { self.state.inner_tm.Activate()? };
        self.state.note_client_id(client_id);
        tsf_step(format!(
            "[tsf] ITfThreadMgr::Activate -> client_id={}",
            client_id
        ));
        Ok(client_id)
    }

    fn Deactivate(&self) -> windows::core::Result<()> {
        let proxy_local = *self.state.proxy_key_sink_only.lock().unwrap();
        tsf_step(format!(
            "[tsf] ITfThreadMgr::Deactivate proxy_key_sink_only={}",
            proxy_local
        ));
        self.state
            .notify_key_sink_focus_local(false, "ThreadMgrProxy::Deactivate");
        self.state.notify_thread_focus_local(false);
        *self.state.foreground_tip.lock().unwrap() = None;
        self.state.notify_profile_activation_local(false);
        unsafe { self.state.inner_tm.Deactivate() }
    }

    fn CreateDocumentMgr(&self) -> windows::core::Result<ITfDocumentMgr> {
        let doc_mgr = unsafe { self.state.inner_tm.CreateDocumentMgr()? };
        self.state.notify_init_document_mgr(&doc_mgr);
        Ok(doc_mgr)
    }

    fn EnumDocumentMgrs(&self) -> windows::core::Result<IEnumTfDocumentMgrs> {
        unsafe { self.state.inner_tm.EnumDocumentMgrs() }
    }

    fn GetFocus(&self) -> windows::core::Result<ITfDocumentMgr> {
        let result = unsafe { self.state.inner_tm.GetFocus() };
        tsf_step(format!(
            "[tsf] ITfThreadMgr::GetFocus -> {:?}",
            result.as_ref().map(|doc| doc.as_raw())
        ));
        result
    }

    fn SetFocus(&self, pdimfocus: Option<&ITfDocumentMgr>) -> windows::core::Result<()> {
        let prev = unsafe { self.state.inner_tm.GetFocus().ok() };
        unsafe { self.state.inner_tm.SetFocus(pdimfocus)? };
        let focused = pdimfocus.is_some();
        let proxy_local = *self.state.proxy_key_sink_only.lock().unwrap();
        tsf_step(format!(
            "[tsf] ITfThreadMgr::SetFocus focused={} proxy_key_sink_only={} prev={:p} new={:p}",
            focused,
            proxy_local,
            prev.as_ref()
                .map(|v| v.as_raw())
                .unwrap_or(std::ptr::null_mut()),
            pdimfocus
                .map(|v| v.as_raw())
                .unwrap_or(std::ptr::null_mut())
        ));
        self.state
            .notify_key_sink_focus_local(focused, "ThreadMgrProxy::SetFocus");
        self.state.notify_thread_focus_local(focused);
        *self.state.foreground_tip.lock().unwrap() = pdimfocus.map(|_| self.state.tip_clsid);
        self.state.notify_profile_activation_local(focused);
        self.state.notify_set_focus(pdimfocus, prev.as_ref());
        Ok(())
    }

    fn AssociateFocus(
        &self,
        hwnd: HWND,
        pdimnew: Option<&ITfDocumentMgr>,
    ) -> windows::core::Result<ITfDocumentMgr> {
        let result = unsafe { self.state.inner_tm.AssociateFocus(hwnd, pdimnew) };
        tsf_step(format!(
            "[tsf] ITfThreadMgr::AssociateFocus hwnd={:?} new={:p} -> {:?}",
            hwnd,
            pdimnew.map(|v| v.as_raw()).unwrap_or(std::ptr::null_mut()),
            result.as_ref().map(|doc| doc.as_raw())
        ));
        result
    }

    fn IsThreadFocus(&self) -> windows::core::Result<BOOL> {
        unsafe { self.state.inner_tm.IsThreadFocus() }
    }

    fn GetFunctionProvider(
        &self,
        clsid: *const GUID,
    ) -> windows::core::Result<ITfFunctionProvider> {
        if let Some(provider) = self.state.find_function_provider(clsid) {
            tsf_step(format!(
                "[tsf] ITfThreadMgr::GetFunctionProvider local-hit clsid={:?}",
                unsafe { clsid.as_ref().copied() }
            ));
            return Ok(provider);
        }
        tsf_warn(format!(
            "[tsf] ITfThreadMgr::GetFunctionProvider fallback-to-inner clsid={:?}",
            unsafe { clsid.as_ref().copied() }
        ));
        let result = unsafe { self.state.inner_tm.GetFunctionProvider(clsid) };
        tsf_step(format!(
            "[tsf] ITfThreadMgr::GetFunctionProvider inner result={:?}",
            result
        ));
        result
    }

    fn EnumFunctionProviders(&self) -> windows::core::Result<IEnumTfFunctionProviders> {
        tsf_step("[tsf] ITfThreadMgr::EnumFunctionProviders");
        let mut providers = self.state.function_providers_snapshot();
        if let Ok(inner_enum) = unsafe { self.state.inner_tm.EnumFunctionProviders() } {
            let mut buf = vec![None::<ITfFunctionProvider>; 8];
            loop {
                let mut fetched = 0u32;
                if unsafe { inner_enum.Next(&mut buf, std::ptr::addr_of_mut!(fetched)) }.is_err() {
                    break;
                }
                if fetched == 0 {
                    break;
                }
                for provider in buf.iter().take(fetched as usize).flatten() {
                    if providers
                        .iter()
                        .all(|existing| existing.as_raw() != provider.as_raw())
                    {
                        providers.push(provider.clone());
                    }
                }
            }
        }
        tsf_step(format!(
            "[tsf] ITfThreadMgr::EnumFunctionProviders merged_count={}",
            providers.len()
        ));
        Ok(FunctionProviderEnum::new(providers).into())
    }

    fn GetGlobalCompartment(&self) -> windows::core::Result<ITfCompartmentMgr> {
        unsafe { self.state.inner_tm.GetGlobalCompartment() }
    }
}

impl ITfThreadMgrEx_Impl for ThreadMgrProxy {
    fn ActivateEx(&self, ptid: *mut u32, dwflags: u32) -> windows::core::Result<()> {
        tsf_step(format!(
            "[tsf] ITfThreadMgrEx::ActivateEx enter ptid={:p} flags=0x{:X}",
            ptid, dwflags
        ));
        unsafe { self.state.inner_tm_ex.ActivateEx(ptid, dwflags)? };
        if !ptid.is_null() {
            unsafe {
                self.state.note_client_id(*ptid);
                tsf_step(format!(
                    "[tsf] ITfThreadMgrEx::ActivateEx exit client_id={}",
                    *ptid
                ));
            }
        } else {
            tsf_step("[tsf] ITfThreadMgrEx::ActivateEx exit without client_id pointer");
        }
        Ok(())
    }

    fn GetActiveFlags(&self) -> windows::core::Result<u32> {
        unsafe { self.state.inner_tm_ex.GetActiveFlags() }
    }
}

impl ITfKeystrokeMgr_Impl for ThreadMgrProxy {
    fn AdviseKeyEventSink(
        &self,
        tid: u32,
        psink: Option<&ITfKeyEventSink>,
        fforeground: BOOL,
    ) -> windows::core::Result<()> {
        if let Some(sink) = psink {
            *self.state.sink.lock().unwrap() = Some((tid, sink.clone()));
            if fforeground.as_bool() {
                *self.state.foreground_tip.lock().unwrap() = Some(self.state.tip_clsid);
            }
            tsf_step(format!(
                "[tsf] ITfKeystrokeMgr::AdviseKeyEventSink tid={} foreground={}",
                tid,
                fforeground.as_bool()
            ));
            let inner_result = unsafe {
                self.state
                    .inner_ks
                    .AdviseKeyEventSink(tid, sink, fforeground)
            };
            tsf_step(format!(
                "[tsf] ITfKeystrokeMgr::AdviseKeyEventSink inner_result={:?}",
                inner_result
            ));
            if inner_result.is_err() {
                *self.state.proxy_key_sink_only.lock().unwrap() = true;
                if fforeground.as_bool() {
                    let has_focus = unsafe { self.state.inner_tm.GetFocus().is_ok() };
                    if has_focus {
                        self.state.notify_key_sink_focus_local(
                            true,
                            "AdviseKeyEventSink(inner rejected, focused)",
                        );
                        self.state.notify_thread_focus_local(true);
                        self.state.notify_profile_activation_local(true);
                    }
                }
                tsf_warn("[tsf] inner ITfKeystrokeMgr::AdviseKeyEventSink rejected; keeping proxy-local foreground state");
            } else {
                *self.state.proxy_key_sink_only.lock().unwrap() = false;
                if fforeground.as_bool() {
                    let has_focus = unsafe { self.state.inner_tm.GetFocus().is_ok() };
                    tsf_step(format!(
                        "[tsf] ITfKeystrokeMgr::AdviseKeyEventSink accepted has_focus={}",
                        has_focus
                    ));
                    if has_focus {
                        self.state.notify_key_sink_focus_local(
                            true,
                            "AdviseKeyEventSink(inner accepted, focused)",
                        );
                        self.state.notify_thread_focus_local(true);
                        self.state.notify_profile_activation_local(true);
                    }
                }
            }
        }
        Ok(())
    }

    fn UnadviseKeyEventSink(&self, tid: u32) -> windows::core::Result<()> {
        let mut sink = self.state.sink.lock().unwrap();
        if sink.as_ref().map(|(sink_tid, _)| *sink_tid) == Some(tid) {
            if *self.state.proxy_key_sink_only.lock().unwrap() {
                if let Some((_, existing)) = sink.as_ref() {
                    unsafe {
                        let _ = existing.OnSetFocus(false);
                    }
                }
            }
            *self.state.foreground_tip.lock().unwrap() = None;
            *self.state.proxy_key_sink_only.lock().unwrap() = false;
            *sink = None;
        }
        let _ = unsafe { self.state.inner_ks.UnadviseKeyEventSink(tid) };
        Ok(())
    }

    fn GetForeground(&self) -> windows::core::Result<GUID> {
        if let Some(clsid) = *self.state.foreground_tip.lock().unwrap() {
            tsf_step(format!(
                "[tsf] ITfKeystrokeMgr::GetForeground local={:?}",
                clsid
            ));
            return Ok(clsid);
        }
        let result = unsafe { self.state.inner_ks.GetForeground() };
        tsf_step(format!(
            "[tsf] ITfKeystrokeMgr::GetForeground inner={:?}",
            result
        ));
        result
    }

    fn TestKeyDown(&self, wparam: WPARAM, lparam: LPARAM) -> windows::core::Result<BOOL> {
        let ctx = self.state.active_context.lock().unwrap().clone();
        let sink = self.state.sink.lock().unwrap().clone();
        if let (Some(ctx), Some((_tid, sink))) = (ctx, sink) {
            let result = unsafe { sink.OnTestKeyDown(&ctx, wparam, lparam) };
            tsf_step(format!(
                "[tsf] ITfKeyEventSink::OnTestKeyDown vk=0x{:X} -> {:?}",
                wparam.0, result
            ));
            return result;
        }
        tsf_warn("[tsf] TestKeyDown without active context or key sink; fallback to inner");
        unsafe { self.state.inner_ks.TestKeyDown(wparam, lparam) }
    }

    fn TestKeyUp(&self, wparam: WPARAM, lparam: LPARAM) -> windows::core::Result<BOOL> {
        let ctx = self.state.active_context.lock().unwrap().clone();
        let sink = self.state.sink.lock().unwrap().clone();
        if let (Some(ctx), Some((_tid, sink))) = (ctx, sink) {
            let result = unsafe { sink.OnTestKeyUp(&ctx, wparam, lparam) };
            tsf_step(format!(
                "[tsf] ITfKeyEventSink::OnTestKeyUp vk=0x{:X} -> {:?}",
                wparam.0, result
            ));
            return result;
        }
        tsf_warn("[tsf] TestKeyUp without active context or key sink; fallback to inner");
        unsafe { self.state.inner_ks.TestKeyUp(wparam, lparam) }
    }

    fn KeyDown(&self, wparam: WPARAM, lparam: LPARAM) -> windows::core::Result<BOOL> {
        let ctx = self.state.active_context.lock().unwrap().clone();
        let sink = self.state.sink.lock().unwrap().clone();
        if let (Some(ctx), Some((_tid, sink))) = (ctx, sink) {
            let result = unsafe { sink.OnKeyDown(&ctx, wparam, lparam) };
            tsf_step(format!(
                "[tsf] ITfKeyEventSink::OnKeyDown vk=0x{:X} -> {:?}",
                wparam.0, result
            ));
            return result;
        }
        tsf_warn("[tsf] KeyDown without active context or key sink; fallback to inner");
        unsafe { self.state.inner_ks.KeyDown(wparam, lparam) }
    }

    fn KeyUp(&self, wparam: WPARAM, lparam: LPARAM) -> windows::core::Result<BOOL> {
        let ctx = self.state.active_context.lock().unwrap().clone();
        let sink = self.state.sink.lock().unwrap().clone();
        if let (Some(ctx), Some((_tid, sink))) = (ctx, sink) {
            let result = unsafe { sink.OnKeyUp(&ctx, wparam, lparam) };
            tsf_step(format!(
                "[tsf] ITfKeyEventSink::OnKeyUp vk=0x{:X} -> {:?}",
                wparam.0, result
            ));
            return result;
        }
        tsf_warn("[tsf] KeyUp without active context or key sink; fallback to inner");
        unsafe { self.state.inner_ks.KeyUp(wparam, lparam) }
    }

    fn GetPreservedKey(
        &self,
        pic: Option<&ITfContext>,
        pprekey: *const TF_PRESERVEDKEY,
    ) -> windows::core::Result<GUID> {
        unsafe { self.state.inner_ks.GetPreservedKey(pic, pprekey) }
    }

    fn IsPreservedKey(
        &self,
        rguid: *const GUID,
        pprekey: *const TF_PRESERVEDKEY,
    ) -> windows::core::Result<BOOL> {
        unsafe { self.state.inner_ks.IsPreservedKey(rguid, pprekey) }
    }

    fn PreserveKey(
        &self,
        tid: u32,
        rguid: *const GUID,
        prekey: *const TF_PRESERVEDKEY,
        pchdesc: &PCWSTR,
        cchdesc: u32,
    ) -> windows::core::Result<()> {
        unsafe {
            let slice = if pchdesc.0.is_null() || cchdesc == 0 {
                &[][..]
            } else {
                std::slice::from_raw_parts(pchdesc.0, cchdesc as usize)
            };
            self.state.inner_ks.PreserveKey(tid, rguid, prekey, slice)
        }
    }

    fn UnpreserveKey(
        &self,
        rguid: *const GUID,
        pprekey: *const TF_PRESERVEDKEY,
    ) -> windows::core::Result<()> {
        unsafe { self.state.inner_ks.UnpreserveKey(rguid, pprekey) }
    }

    fn SetPreservedKeyDescription(
        &self,
        rguid: *const GUID,
        pchdesc: &PCWSTR,
        cchdesc: u32,
    ) -> windows::core::Result<()> {
        unsafe {
            let slice = if pchdesc.0.is_null() || cchdesc == 0 {
                &[][..]
            } else {
                std::slice::from_raw_parts(pchdesc.0, cchdesc as usize)
            };
            self.state.inner_ks.SetPreservedKeyDescription(rguid, slice)
        }
    }

    fn GetPreservedKeyDescription(&self, rguid: *const GUID) -> windows::core::Result<BSTR> {
        unsafe { self.state.inner_ks.GetPreservedKeyDescription(rguid) }
    }

    fn SimulatePreservedKey(
        &self,
        pic: Option<&ITfContext>,
        rguid: *const GUID,
    ) -> windows::core::Result<BOOL> {
        unsafe { self.state.inner_ks.SimulatePreservedKey(pic, rguid) }
    }
}

impl ITfSource_Impl for ThreadMgrProxy {
    fn AdviseSink(
        &self,
        riid: *const GUID,
        punk: Option<&windows::core::IUnknown>,
    ) -> windows::core::Result<u32> {
        if riid.is_null() || punk.is_none() {
            return Err(E_INVALIDARG.into());
        }
        let riid_value = unsafe { *riid };
        let punk = punk.unwrap();
        tsf_step(format!(
            "[tsf] ITfSource::AdviseSink riid={:?} ({}) punk={:p}",
            riid_value,
            describe_source_iid(&riid_value),
            punk.as_raw()
        ));

        if riid_value == ITfThreadMgrEventSink::IID {
            let sink: ITfThreadMgrEventSink = punk.cast()?;
            *self.state.thread_mgr_event_sink.lock().unwrap() = Some(sink);
            tsf_step("[tsf] ITfSource::AdviseSink ITfThreadMgrEventSink");
            return Ok(0x534F_474F);
        }

        if riid_value == ITfThreadFocusSink::IID {
            let sink: ITfThreadFocusSink = punk.cast()?;
            *self.state.thread_focus_sink.lock().unwrap() = Some(sink);
            tsf_step("[tsf] ITfSource::AdviseSink ITfThreadFocusSink");
            return Ok(0x534F_4754);
        }

        if riid_value == ITfActiveLanguageProfileNotifySink::IID {
            let sink: ITfActiveLanguageProfileNotifySink = punk.cast()?;
            *self.state.active_lang_profile_sink.lock().unwrap() = Some(sink);
            tsf_step("[tsf] ITfSource::AdviseSink ITfActiveLanguageProfileNotifySink");
            return Ok(0x534F_4752);
        }

        if riid_value == ITfInputProcessorProfileActivationSink::IID {
            let sink: ITfInputProcessorProfileActivationSink = punk.cast()?;
            *self.state.profile_activation_sink.lock().unwrap() = Some(sink);
            tsf_step("[tsf] ITfSource::AdviseSink ITfInputProcessorProfileActivationSink");
            return Ok(0x534F_4753);
        }

        if riid_value == ITfUIElementSink::IID {
            let sink: ITfUIElementSink = punk.cast()?;
            *self.state.ui_element_sink.lock().unwrap() = Some(sink);
            tsf_step("[tsf] ITfSource::AdviseSink ITfUIElementSink");
            return Ok(0x534F_4751);
        }

        let result = unsafe { self.state.inner_source.AdviseSink(riid, punk) };
        tsf_step(format!(
            "[tsf] ITfSource::AdviseSink fallback result={:?}",
            result
        ));
        result
    }

    fn UnadviseSink(&self, dwcookie: u32) -> windows::core::Result<()> {
        if dwcookie == 0x534F_474F {
            *self.state.thread_mgr_event_sink.lock().unwrap() = None;
            return Ok(());
        }
        if dwcookie == 0x534F_4754 {
            *self.state.thread_focus_sink.lock().unwrap() = None;
            return Ok(());
        }
        if dwcookie == 0x534F_4752 {
            *self.state.active_lang_profile_sink.lock().unwrap() = None;
            return Ok(());
        }
        if dwcookie == 0x534F_4753 {
            *self.state.profile_activation_sink.lock().unwrap() = None;
            return Ok(());
        }
        if dwcookie == 0x534F_4751 {
            *self.state.ui_element_sink.lock().unwrap() = None;
            return Ok(());
        }
        unsafe { self.state.inner_source.UnadviseSink(dwcookie) }
    }
}

impl ITfSourceSingle_Impl for ThreadMgrProxy {
    fn AdviseSingleSink(
        &self,
        tid: u32,
        riid: *const GUID,
        punk: Option<&windows::core::IUnknown>,
    ) -> windows::core::Result<()> {
        if riid.is_null() || punk.is_none() {
            return Err(E_INVALIDARG.into());
        }
        let riid_value = unsafe { *riid };
        let punk = punk.unwrap().clone();
        tsf_step(format!(
            "[tsf] ITfSourceSingle::AdviseSingleSink enter tid={} riid={:?} punk={:p}",
            tid,
            riid_value,
            punk.as_raw()
        ));

        if riid_value == ITfFunctionProvider::IID {
            let provider: ITfFunctionProvider = punk.cast()?;
            self.state.register_function_provider(tid, provider);
            self.state
                .register_source_single_sink(tid, riid_value, punk);
            tsf_step("[tsf] ITfSourceSingle::AdviseSingleSink stored ITfFunctionProvider");
            return Ok(());
        }

        tsf_warn(format!(
            "[tsf] ITfSourceSingle::AdviseSingleSink tid={} riid={:?} -> keep sink locally",
            tid, riid_value
        ));
        self.state
            .register_source_single_sink(tid, riid_value, punk);
        Ok(())
    }

    fn UnadviseSingleSink(&self, tid: u32, riid: *const GUID) -> windows::core::Result<()> {
        if riid.is_null() {
            return Err(E_INVALIDARG.into());
        }
        let riid_value = unsafe { *riid };
        self.state.unregister_function_provider(tid, &riid_value);
        self.state.unregister_source_single_sink(tid, &riid_value);
        Ok(())
    }
}

impl ITfCompartmentMgr_Impl for ThreadMgrProxy {
    fn GetCompartment(&self, rguid: *const GUID) -> windows::core::Result<ITfCompartment> {
        unsafe { self.state.inner_compartment_mgr.GetCompartment(rguid) }
    }

    fn ClearCompartment(&self, tid: u32, rguid: *const GUID) -> windows::core::Result<()> {
        unsafe {
            self.state
                .inner_compartment_mgr
                .ClearCompartment(tid, rguid)
        }
    }

    fn EnumCompartments(&self) -> windows::core::Result<IEnumGUID> {
        unsafe { self.state.inner_compartment_mgr.EnumCompartments() }
    }
}

impl ITfLangBarItemMgr_Impl for ThreadMgrProxy {
    fn EnumItems(&self) -> windows::core::Result<IEnumTfLangBarItems> {
        tsf_step(format!(
            "[tsf] ITfLangBarItemMgr::EnumItems backend={}",
            if self.state.inner_lang_bar_item_mgr.is_some() {
                "inner"
            } else {
                "fallback"
            }
        ));
        if let Some(inner) = &self.state.inner_lang_bar_item_mgr {
            let result = unsafe { inner.EnumItems() };
            tsf_step(format!(
                "[tsf] ITfLangBarItemMgr::EnumItems inner result={:?}",
                result
            ));
            result
        } else {
            let items = self.state.local_lang_bar_items_snapshot();
            tsf_step(format!(
                "[tsf] ITfLangBarItemMgr::EnumItems local count={}",
                items.len()
            ));
            Ok(LangBarItemEnum::new(items).into())
        }
    }

    fn GetItem(&self, rguid: *const GUID) -> windows::core::Result<ITfLangBarItem> {
        let guid = unsafe { rguid.as_ref().copied() };
        tsf_step(format!(
            "[tsf] ITfLangBarItemMgr::GetItem guid={:?} backend={}",
            guid,
            if self.state.inner_lang_bar_item_mgr.is_some() {
                "inner"
            } else {
                "fallback"
            }
        ));
        if let Some(inner) = &self.state.inner_lang_bar_item_mgr {
            let result = unsafe { inner.GetItem(rguid) };
            tsf_step(format!(
                "[tsf] ITfLangBarItemMgr::GetItem inner result={:?}",
                result
            ));
            result
        } else {
            self.state
                .get_local_lang_bar_item(rguid)
                .ok_or_else(|| E_NOINTERFACE.into())
        }
    }

    fn AddItem(&self, punk: Option<&ITfLangBarItem>) -> windows::core::Result<()> {
        if let Some(inner) = &self.state.inner_lang_bar_item_mgr {
            unsafe { inner.AddItem(punk) }
        } else {
            if let Some(item) = punk {
                self.state.add_local_lang_bar_item(item);
            }
            Ok(())
        }
    }

    fn RemoveItem(&self, punk: Option<&ITfLangBarItem>) -> windows::core::Result<()> {
        if let Some(inner) = &self.state.inner_lang_bar_item_mgr {
            unsafe { inner.RemoveItem(punk) }
        } else {
            if let Some(item) = punk {
                self.state.remove_local_lang_bar_item(item);
            }
            Ok(())
        }
    }

    fn AdviseItemSink(
        &self,
        punk: Option<&ITfLangBarItemSink>,
        pdwcookie: *mut u32,
        rguiditem: *const GUID,
    ) -> windows::core::Result<()> {
        if let Some(inner) = &self.state.inner_lang_bar_item_mgr {
            unsafe { inner.AdviseItemSink(punk, pdwcookie, rguiditem) }
        } else {
            if pdwcookie.is_null() {
                return Err(E_INVALIDARG.into());
            }
            unsafe {
                *pdwcookie = 1;
            }
            Ok(())
        }
    }

    fn UnadviseItemSink(&self, dwcookie: u32) -> windows::core::Result<()> {
        if let Some(inner) = &self.state.inner_lang_bar_item_mgr {
            unsafe { inner.UnadviseItemSink(dwcookie) }
        } else {
            Ok(())
        }
    }

    fn GetItemFloatingRect(
        &self,
        dwthreadid: u32,
        rguid: *const GUID,
    ) -> windows::core::Result<RECT> {
        if let Some(inner) = &self.state.inner_lang_bar_item_mgr {
            unsafe { inner.GetItemFloatingRect(dwthreadid, rguid) }
        } else {
            Ok(RECT::default())
        }
    }

    fn GetItemsStatus(
        &self,
        ulcount: u32,
        prgguid: *const GUID,
        pdwstatus: *mut u32,
    ) -> windows::core::Result<()> {
        let guids = if prgguid.is_null() || ulcount == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(prgguid, ulcount as usize) }.to_vec()
        };
        tsf_step(format!(
            "[tsf] ITfLangBarItemMgr::GetItemsStatus count={} guids={:?} backend={}",
            ulcount,
            guids,
            if self.state.inner_lang_bar_item_mgr.is_some() {
                "inner"
            } else {
                "fallback"
            }
        ));
        if let Some(inner) = &self.state.inner_lang_bar_item_mgr {
            let result = unsafe { inner.GetItemsStatus(ulcount, prgguid, pdwstatus) };
            tsf_step(format!(
                "[tsf] ITfLangBarItemMgr::GetItemsStatus inner result={:?}",
                result
            ));
            result
        } else {
            if pdwstatus.is_null() {
                return Err(E_INVALIDARG.into());
            }
            unsafe {
                for i in 0..ulcount as usize {
                    pdwstatus.add(i).write(0);
                }
            }
            Ok(())
        }
    }

    fn GetItemNum(&self) -> windows::core::Result<u32> {
        tsf_step(format!(
            "[tsf] ITfLangBarItemMgr::GetItemNum backend={}",
            if self.state.inner_lang_bar_item_mgr.is_some() {
                "inner"
            } else {
                "fallback"
            }
        ));
        if let Some(inner) = &self.state.inner_lang_bar_item_mgr {
            let result = unsafe { inner.GetItemNum() };
            tsf_step(format!(
                "[tsf] ITfLangBarItemMgr::GetItemNum inner result={:?}",
                result
            ));
            result
        } else {
            Ok(self.state.local_lang_bar_items.lock().unwrap().len() as u32)
        }
    }

    fn GetItems(
        &self,
        ulcount: u32,
        ppitem: *mut Option<ITfLangBarItem>,
        pinfo: *mut TF_LANGBARITEMINFO,
        pdwstatus: *mut u32,
        pcfetched: *mut u32,
    ) -> windows::core::Result<()> {
        tsf_step(format!(
            "[tsf] ITfLangBarItemMgr::GetItems count={} backend={}",
            ulcount,
            if self.state.inner_lang_bar_item_mgr.is_some() {
                "inner"
            } else {
                "fallback"
            }
        ));
        if let Some(inner) = &self.state.inner_lang_bar_item_mgr {
            let result = unsafe { inner.GetItems(ulcount, ppitem, pinfo, pdwstatus, pcfetched) };
            tsf_step(format!(
                "[tsf] ITfLangBarItemMgr::GetItems inner result={:?}",
                result
            ));
            result
        } else {
            if ppitem.is_null() || pinfo.is_null() || pdwstatus.is_null() {
                return Err(E_INVALIDARG.into());
            }
            let items = self.state.local_lang_bar_items.lock().unwrap();
            let fetched = items.len().min(ulcount as usize);
            unsafe {
                for i in 0..fetched {
                    let item = &items[i].item;
                    let mut info = std::mem::zeroed::<TF_LANGBARITEMINFO>();
                    let _ = item.GetInfo(&mut info);
                    let status = item.GetStatus().unwrap_or(0);
                    ppitem.add(i).write(Some(item.clone()));
                    pinfo.add(i).write(info);
                    pdwstatus.add(i).write(status);
                }
                for i in fetched..ulcount as usize {
                    ppitem.add(i).write(None);
                    pinfo.add(i).write(std::mem::zeroed());
                    pdwstatus.add(i).write(0);
                }
                if !pcfetched.is_null() {
                    *pcfetched = fetched as u32;
                }
            }
            if fetched == ulcount as usize {
                Ok(())
            } else {
                Err(S_FALSE.into())
            }
        }
    }

    fn AdviseItemsSink(
        &self,
        ulcount: u32,
        ppunk: *const Option<ITfLangBarItemSink>,
        pguiditem: *const GUID,
        pdwcookie: *mut u32,
    ) -> windows::core::Result<()> {
        let guids = if pguiditem.is_null() || ulcount == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(pguiditem, ulcount as usize) }.to_vec()
        };
        tsf_step(format!(
            "[tsf] ITfLangBarItemMgr::AdviseItemsSink count={} guids={:?} backend={}",
            ulcount,
            guids,
            if self.state.inner_lang_bar_item_mgr.is_some() {
                "inner"
            } else {
                "fallback"
            }
        ));
        if let Some(inner) = &self.state.inner_lang_bar_item_mgr {
            let result = unsafe { inner.AdviseItemsSink(ulcount, ppunk, pguiditem, pdwcookie) };
            tsf_step(format!(
                "[tsf] ITfLangBarItemMgr::AdviseItemsSink inner result={:?}",
                result
            ));
            result
        } else {
            if pdwcookie.is_null() {
                return Err(E_INVALIDARG.into());
            }
            unsafe {
                for i in 0..ulcount as usize {
                    pdwcookie.add(i).write((i + 1) as u32);
                }
            }
            Ok(())
        }
    }

    fn UnadviseItemsSink(&self, ulcount: u32, pdwcookie: *const u32) -> windows::core::Result<()> {
        if let Some(inner) = &self.state.inner_lang_bar_item_mgr {
            unsafe {
                let slice = if pdwcookie.is_null() || ulcount == 0 {
                    &[][..]
                } else {
                    std::slice::from_raw_parts(pdwcookie, ulcount as usize)
                };
                inner.UnadviseItemsSink(slice)
            }
        } else {
            Ok(())
        }
    }
}

impl ITfMessagePump_Impl for ThreadMgrProxy {
    fn PeekMessageA(
        &self,
        pmsg: *mut windows::Win32::UI::WindowsAndMessaging::MSG,
        hwnd: HWND,
        wmsgfiltermin: u32,
        wmsgfiltermax: u32,
        wremovemsg: u32,
        pfresult: *mut BOOL,
    ) -> windows::core::Result<()> {
        unsafe {
            self.state.inner_message_pump.PeekMessageA(
                pmsg,
                hwnd,
                wmsgfiltermin,
                wmsgfiltermax,
                wremovemsg,
                pfresult,
            )
        }
    }

    fn GetMessageA(
        &self,
        pmsg: *mut windows::Win32::UI::WindowsAndMessaging::MSG,
        hwnd: HWND,
        wmsgfiltermin: u32,
        wmsgfiltermax: u32,
        pfresult: *mut BOOL,
    ) -> windows::core::Result<()> {
        unsafe {
            self.state.inner_message_pump.GetMessageA(
                pmsg,
                hwnd,
                wmsgfiltermin,
                wmsgfiltermax,
                pfresult,
            )
        }
    }

    fn PeekMessageW(
        &self,
        pmsg: *mut windows::Win32::UI::WindowsAndMessaging::MSG,
        hwnd: HWND,
        wmsgfiltermin: u32,
        wmsgfiltermax: u32,
        wremovemsg: u32,
        pfresult: *mut BOOL,
    ) -> windows::core::Result<()> {
        unsafe {
            self.state.inner_message_pump.PeekMessageW(
                pmsg,
                hwnd,
                wmsgfiltermin,
                wmsgfiltermax,
                wremovemsg,
                pfresult,
            )
        }
    }

    fn GetMessageW(
        &self,
        pmsg: *mut windows::Win32::UI::WindowsAndMessaging::MSG,
        hwnd: HWND,
        wmsgfiltermin: u32,
        wmsgfiltermax: u32,
        pfresult: *mut BOOL,
    ) -> windows::core::Result<()> {
        unsafe {
            self.state.inner_message_pump.GetMessageW(
                pmsg,
                hwnd,
                wmsgfiltermin,
                wmsgfiltermax,
                pfresult,
            )
        }
    }
}

impl ITfClientId_Impl for ThreadMgrProxy {
    fn GetClientId(&self, rclsid: *const GUID) -> windows::core::Result<u32> {
        let result = (*self.state.client_id.lock().unwrap()).ok_or_else(|| E_NOTIMPL.into());
        tsf_step(format!(
            "[tsf] ITfClientId::GetClientId rclsid={:?} -> {:?}",
            unsafe { rclsid.as_ref().copied() },
            result
        ));
        result
    }
}

impl ITfUIElementMgr_Impl for ThreadMgrProxy {
    fn BeginUIElement(
        &self,
        pelement: Option<&ITfUIElement>,
        pbshow: *mut BOOL,
        pdwuielementid: *mut u32,
    ) -> windows::core::Result<()> {
        let Some(element) = pelement else {
            return Err(E_INVALIDARG.into());
        };
        self.state.ui_element_begin(element, pbshow, pdwuielementid);
        Ok(())
    }

    fn UpdateUIElement(&self, dwuielementid: u32) -> windows::core::Result<()> {
        self.state.ui_element_update(dwuielementid);
        Ok(())
    }

    fn EndUIElement(&self, dwuielementid: u32) -> windows::core::Result<()> {
        self.state.ui_element_end(dwuielementid);
        Ok(())
    }

    fn GetUIElement(&self, dwuielementid: u32) -> windows::core::Result<ITfUIElement> {
        self.state
            .ui_element_get(dwuielementid)
            .ok_or_else(|| E_INVALIDARG.into())
    }

    fn EnumUIElements(&self) -> windows::core::Result<IEnumTfUIElements> {
        Ok(UiElementEnum::new(self.state.ui_element_snapshot()).into())
    }
}

impl ITfThreadMgr_Impl for ThreadMgrProxy_Impl {
    fn Activate(&self) -> windows::core::Result<u32> {
        ITfThreadMgr_Impl::Activate(&self.this)
    }
    fn Deactivate(&self) -> windows::core::Result<()> {
        ITfThreadMgr_Impl::Deactivate(&self.this)
    }
    fn CreateDocumentMgr(&self) -> windows::core::Result<ITfDocumentMgr> {
        ITfThreadMgr_Impl::CreateDocumentMgr(&self.this)
    }
    fn EnumDocumentMgrs(&self) -> windows::core::Result<IEnumTfDocumentMgrs> {
        ITfThreadMgr_Impl::EnumDocumentMgrs(&self.this)
    }
    fn GetFocus(&self) -> windows::core::Result<ITfDocumentMgr> {
        ITfThreadMgr_Impl::GetFocus(&self.this)
    }
    fn SetFocus(&self, pdimfocus: Option<&ITfDocumentMgr>) -> windows::core::Result<()> {
        ITfThreadMgr_Impl::SetFocus(&self.this, pdimfocus)
    }
    fn AssociateFocus(
        &self,
        hwnd: HWND,
        pdimnew: Option<&ITfDocumentMgr>,
    ) -> windows::core::Result<ITfDocumentMgr> {
        ITfThreadMgr_Impl::AssociateFocus(&self.this, hwnd, pdimnew)
    }
    fn IsThreadFocus(&self) -> windows::core::Result<BOOL> {
        ITfThreadMgr_Impl::IsThreadFocus(&self.this)
    }
    fn GetFunctionProvider(
        &self,
        clsid: *const GUID,
    ) -> windows::core::Result<ITfFunctionProvider> {
        ITfThreadMgr_Impl::GetFunctionProvider(&self.this, clsid)
    }
    fn EnumFunctionProviders(&self) -> windows::core::Result<IEnumTfFunctionProviders> {
        ITfThreadMgr_Impl::EnumFunctionProviders(&self.this)
    }
    fn GetGlobalCompartment(&self) -> windows::core::Result<ITfCompartmentMgr> {
        ITfThreadMgr_Impl::GetGlobalCompartment(&self.this)
    }
}

impl ITfThreadMgrEx_Impl for ThreadMgrProxy_Impl {
    fn ActivateEx(&self, ptid: *mut u32, dwflags: u32) -> windows::core::Result<()> {
        ITfThreadMgrEx_Impl::ActivateEx(&self.this, ptid, dwflags)
    }
    fn GetActiveFlags(&self) -> windows::core::Result<u32> {
        ITfThreadMgrEx_Impl::GetActiveFlags(&self.this)
    }
}

impl ITfKeystrokeMgr_Impl for ThreadMgrProxy_Impl {
    fn AdviseKeyEventSink(
        &self,
        tid: u32,
        psink: Option<&ITfKeyEventSink>,
        fforeground: BOOL,
    ) -> windows::core::Result<()> {
        ITfKeystrokeMgr_Impl::AdviseKeyEventSink(&self.this, tid, psink, fforeground)
    }
    fn UnadviseKeyEventSink(&self, tid: u32) -> windows::core::Result<()> {
        ITfKeystrokeMgr_Impl::UnadviseKeyEventSink(&self.this, tid)
    }
    fn GetForeground(&self) -> windows::core::Result<GUID> {
        ITfKeystrokeMgr_Impl::GetForeground(&self.this)
    }
    fn TestKeyDown(&self, wparam: WPARAM, lparam: LPARAM) -> windows::core::Result<BOOL> {
        ITfKeystrokeMgr_Impl::TestKeyDown(&self.this, wparam, lparam)
    }
    fn TestKeyUp(&self, wparam: WPARAM, lparam: LPARAM) -> windows::core::Result<BOOL> {
        ITfKeystrokeMgr_Impl::TestKeyUp(&self.this, wparam, lparam)
    }
    fn KeyDown(&self, wparam: WPARAM, lparam: LPARAM) -> windows::core::Result<BOOL> {
        ITfKeystrokeMgr_Impl::KeyDown(&self.this, wparam, lparam)
    }
    fn KeyUp(&self, wparam: WPARAM, lparam: LPARAM) -> windows::core::Result<BOOL> {
        ITfKeystrokeMgr_Impl::KeyUp(&self.this, wparam, lparam)
    }
    fn GetPreservedKey(
        &self,
        pic: Option<&ITfContext>,
        pprekey: *const TF_PRESERVEDKEY,
    ) -> windows::core::Result<GUID> {
        ITfKeystrokeMgr_Impl::GetPreservedKey(&self.this, pic, pprekey)
    }
    fn IsPreservedKey(
        &self,
        rguid: *const GUID,
        pprekey: *const TF_PRESERVEDKEY,
    ) -> windows::core::Result<BOOL> {
        ITfKeystrokeMgr_Impl::IsPreservedKey(&self.this, rguid, pprekey)
    }
    fn PreserveKey(
        &self,
        tid: u32,
        rguid: *const GUID,
        prekey: *const TF_PRESERVEDKEY,
        pchdesc: &PCWSTR,
        cchdesc: u32,
    ) -> windows::core::Result<()> {
        ITfKeystrokeMgr_Impl::PreserveKey(&self.this, tid, rguid, prekey, pchdesc, cchdesc)
    }
    fn UnpreserveKey(
        &self,
        rguid: *const GUID,
        pprekey: *const TF_PRESERVEDKEY,
    ) -> windows::core::Result<()> {
        ITfKeystrokeMgr_Impl::UnpreserveKey(&self.this, rguid, pprekey)
    }
    fn SetPreservedKeyDescription(
        &self,
        rguid: *const GUID,
        pchdesc: &PCWSTR,
        cchdesc: u32,
    ) -> windows::core::Result<()> {
        ITfKeystrokeMgr_Impl::SetPreservedKeyDescription(&self.this, rguid, pchdesc, cchdesc)
    }
    fn GetPreservedKeyDescription(&self, rguid: *const GUID) -> windows::core::Result<BSTR> {
        ITfKeystrokeMgr_Impl::GetPreservedKeyDescription(&self.this, rguid)
    }
    fn SimulatePreservedKey(
        &self,
        pic: Option<&ITfContext>,
        rguid: *const GUID,
    ) -> windows::core::Result<BOOL> {
        ITfKeystrokeMgr_Impl::SimulatePreservedKey(&self.this, pic, rguid)
    }
}

impl ITfSource_Impl for ThreadMgrProxy_Impl {
    fn AdviseSink(
        &self,
        riid: *const GUID,
        punk: Option<&windows::core::IUnknown>,
    ) -> windows::core::Result<u32> {
        ITfSource_Impl::AdviseSink(&self.this, riid, punk)
    }
    fn UnadviseSink(&self, dwcookie: u32) -> windows::core::Result<()> {
        ITfSource_Impl::UnadviseSink(&self.this, dwcookie)
    }
}

impl ITfSourceSingle_Impl for ThreadMgrProxy_Impl {
    fn AdviseSingleSink(
        &self,
        tid: u32,
        riid: *const GUID,
        punk: Option<&windows::core::IUnknown>,
    ) -> windows::core::Result<()> {
        ITfSourceSingle_Impl::AdviseSingleSink(&self.this, tid, riid, punk)
    }
    fn UnadviseSingleSink(&self, tid: u32, riid: *const GUID) -> windows::core::Result<()> {
        ITfSourceSingle_Impl::UnadviseSingleSink(&self.this, tid, riid)
    }
}

impl ITfCompartmentMgr_Impl for ThreadMgrProxy_Impl {
    fn GetCompartment(&self, rguid: *const GUID) -> windows::core::Result<ITfCompartment> {
        ITfCompartmentMgr_Impl::GetCompartment(&self.this, rguid)
    }
    fn ClearCompartment(&self, tid: u32, rguid: *const GUID) -> windows::core::Result<()> {
        ITfCompartmentMgr_Impl::ClearCompartment(&self.this, tid, rguid)
    }
    fn EnumCompartments(&self) -> windows::core::Result<IEnumGUID> {
        ITfCompartmentMgr_Impl::EnumCompartments(&self.this)
    }
}

impl ITfLangBarItemMgr_Impl for ThreadMgrProxy_Impl {
    fn EnumItems(&self) -> windows::core::Result<IEnumTfLangBarItems> {
        ITfLangBarItemMgr_Impl::EnumItems(&self.this)
    }

    fn GetItem(&self, rguid: *const GUID) -> windows::core::Result<ITfLangBarItem> {
        ITfLangBarItemMgr_Impl::GetItem(&self.this, rguid)
    }

    fn AddItem(&self, punk: Option<&ITfLangBarItem>) -> windows::core::Result<()> {
        ITfLangBarItemMgr_Impl::AddItem(&self.this, punk)
    }

    fn RemoveItem(&self, punk: Option<&ITfLangBarItem>) -> windows::core::Result<()> {
        ITfLangBarItemMgr_Impl::RemoveItem(&self.this, punk)
    }

    fn AdviseItemSink(
        &self,
        punk: Option<&ITfLangBarItemSink>,
        pdwcookie: *mut u32,
        rguiditem: *const GUID,
    ) -> windows::core::Result<()> {
        ITfLangBarItemMgr_Impl::AdviseItemSink(&self.this, punk, pdwcookie, rguiditem)
    }

    fn UnadviseItemSink(&self, dwcookie: u32) -> windows::core::Result<()> {
        ITfLangBarItemMgr_Impl::UnadviseItemSink(&self.this, dwcookie)
    }

    fn GetItemFloatingRect(
        &self,
        dwthreadid: u32,
        rguid: *const GUID,
    ) -> windows::core::Result<RECT> {
        ITfLangBarItemMgr_Impl::GetItemFloatingRect(&self.this, dwthreadid, rguid)
    }

    fn GetItemsStatus(
        &self,
        ulcount: u32,
        prgguid: *const GUID,
        pdwstatus: *mut u32,
    ) -> windows::core::Result<()> {
        ITfLangBarItemMgr_Impl::GetItemsStatus(&self.this, ulcount, prgguid, pdwstatus)
    }

    fn GetItemNum(&self) -> windows::core::Result<u32> {
        ITfLangBarItemMgr_Impl::GetItemNum(&self.this)
    }

    fn GetItems(
        &self,
        ulcount: u32,
        ppitem: *mut Option<ITfLangBarItem>,
        pinfo: *mut TF_LANGBARITEMINFO,
        pdwstatus: *mut u32,
        pcfetched: *mut u32,
    ) -> windows::core::Result<()> {
        ITfLangBarItemMgr_Impl::GetItems(&self.this, ulcount, ppitem, pinfo, pdwstatus, pcfetched)
    }

    fn AdviseItemsSink(
        &self,
        ulcount: u32,
        ppunk: *const Option<ITfLangBarItemSink>,
        pguiditem: *const GUID,
        pdwcookie: *mut u32,
    ) -> windows::core::Result<()> {
        ITfLangBarItemMgr_Impl::AdviseItemsSink(&self.this, ulcount, ppunk, pguiditem, pdwcookie)
    }

    fn UnadviseItemsSink(&self, ulcount: u32, pdwcookie: *const u32) -> windows::core::Result<()> {
        ITfLangBarItemMgr_Impl::UnadviseItemsSink(&self.this, ulcount, pdwcookie)
    }
}

impl ITfMessagePump_Impl for ThreadMgrProxy_Impl {
    fn PeekMessageA(
        &self,
        pmsg: *mut windows::Win32::UI::WindowsAndMessaging::MSG,
        hwnd: HWND,
        wmsgfiltermin: u32,
        wmsgfiltermax: u32,
        wremovemsg: u32,
        pfresult: *mut BOOL,
    ) -> windows::core::Result<()> {
        ITfMessagePump_Impl::PeekMessageA(
            &self.this,
            pmsg,
            hwnd,
            wmsgfiltermin,
            wmsgfiltermax,
            wremovemsg,
            pfresult,
        )
    }
    fn GetMessageA(
        &self,
        pmsg: *mut windows::Win32::UI::WindowsAndMessaging::MSG,
        hwnd: HWND,
        wmsgfiltermin: u32,
        wmsgfiltermax: u32,
        pfresult: *mut BOOL,
    ) -> windows::core::Result<()> {
        ITfMessagePump_Impl::GetMessageA(
            &self.this,
            pmsg,
            hwnd,
            wmsgfiltermin,
            wmsgfiltermax,
            pfresult,
        )
    }
    fn PeekMessageW(
        &self,
        pmsg: *mut windows::Win32::UI::WindowsAndMessaging::MSG,
        hwnd: HWND,
        wmsgfiltermin: u32,
        wmsgfiltermax: u32,
        wremovemsg: u32,
        pfresult: *mut BOOL,
    ) -> windows::core::Result<()> {
        ITfMessagePump_Impl::PeekMessageW(
            &self.this,
            pmsg,
            hwnd,
            wmsgfiltermin,
            wmsgfiltermax,
            wremovemsg,
            pfresult,
        )
    }
    fn GetMessageW(
        &self,
        pmsg: *mut windows::Win32::UI::WindowsAndMessaging::MSG,
        hwnd: HWND,
        wmsgfiltermin: u32,
        wmsgfiltermax: u32,
        pfresult: *mut BOOL,
    ) -> windows::core::Result<()> {
        ITfMessagePump_Impl::GetMessageW(
            &self.this,
            pmsg,
            hwnd,
            wmsgfiltermin,
            wmsgfiltermax,
            pfresult,
        )
    }
}

impl ITfClientId_Impl for ThreadMgrProxy_Impl {
    fn GetClientId(&self, rclsid: *const GUID) -> windows::core::Result<u32> {
        ITfClientId_Impl::GetClientId(&self.this, rclsid)
    }
}

impl ITfUIElementMgr_Impl for ThreadMgrProxy_Impl {
    fn BeginUIElement(
        &self,
        pelement: Option<&ITfUIElement>,
        pbshow: *mut BOOL,
        pdwuielementid: *mut u32,
    ) -> windows::core::Result<()> {
        ITfUIElementMgr_Impl::BeginUIElement(&self.this, pelement, pbshow, pdwuielementid)
    }
    fn UpdateUIElement(&self, dwuielementid: u32) -> windows::core::Result<()> {
        ITfUIElementMgr_Impl::UpdateUIElement(&self.this, dwuielementid)
    }
    fn EndUIElement(&self, dwuielementid: u32) -> windows::core::Result<()> {
        ITfUIElementMgr_Impl::EndUIElement(&self.this, dwuielementid)
    }
    fn GetUIElement(&self, dwuielementid: u32) -> windows::core::Result<ITfUIElement> {
        ITfUIElementMgr_Impl::GetUIElement(&self.this, dwuielementid)
    }
    fn EnumUIElements(&self) -> windows::core::Result<IEnumTfUIElements> {
        ITfUIElementMgr_Impl::EnumUIElements(&self.this)
    }
}
