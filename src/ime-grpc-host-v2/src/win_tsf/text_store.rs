//! Minimal `ITextStoreACP` backed by a UTF-16 buffer (ACP = UTF-16 code unit index).

use std::collections::VecDeque;
use std::mem::ManuallyDrop;
use std::sync::{Arc, Mutex, Once};
use windows::core::implement;
use windows::core::{w, Interface, GUID, PCWSTR, VARIANT};
use windows::Win32::Foundation::{GetLastError, BOOL, E_NOTIMPL, HINSTANCE, HWND, RECT};
use windows::Win32::System::Com::{IDataObject, FORMATETC};
use windows::Win32::UI::TextServices::ITextStoreACP;
use windows::Win32::UI::TextServices::{
    ITextStoreACPSink, ITextStoreACP_Impl, ITfCompositionView, ITfContextOwnerCompositionSink,
    ITfContextOwnerCompositionSink_Impl, ITfRange, GUID_PROP_ATTRIBUTE, GUID_PROP_COMPOSING,
    GUID_PROP_LANGID, GUID_PROP_READING, GUID_PROP_TEXTOWNER, TEXT_STORE_LOCK_FLAGS, TS_AE_NONE,
    TS_ATTRVAL, TS_ATTR_FIND_WANT_VALUE, TS_IAS_NOQUERY, TS_IAS_QUERYONLY, TS_LC_CHANGE,
    TS_RUNINFO, TS_SELECTIONSTYLE, TS_SELECTION_ACP, TS_SS_NOHIDDENTEXT, TS_STATUS,
    TS_ST_CORRECTION, TS_ST_NONE, TS_TEXTCHANGE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowRect, RegisterClassW, HWND_MESSAGE,
    WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW,
};

use super::diag::tsf_step;

unsafe extern "system" fn text_store_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

static REGISTER_CLASS: Once = Once::new();

fn ensure_window_class() {
    REGISTER_CLASS.call_once(|| unsafe {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(text_store_wndproc),
            lpszClassName: w!("TsfHostTextStoreWindow").into(),
            hInstance: windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                .unwrap_or_default()
                .into(),
            ..Default::default()
        };
        let atom = RegisterClassW(&wc);
        if atom == 0 {
            tsf_step("[tsf-text] RegisterClassW(TsfHostTextStoreWindow) returned 0");
        }
    });
}

fn create_host_window() -> HWND {
    ensure_window_class();
    unsafe {
        let hinstance: HINSTANCE = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
            .unwrap_or_default()
            .into();
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("TsfHostTextStoreWindow"),
            w!("TsfHostTextStoreWindow"),
            WINDOW_STYLE(0),
            100,
            100,
            800,
            600,
            None,
            None,
            hinstance,
            None,
        )
        .unwrap_or_default();
        if !hwnd.is_invalid() {
            tsf_step(format!("[tsf-text] create_host_window hwnd={:?}", hwnd));
            return hwnd;
        }
        tsf_step(format!(
            "[tsf-text] create_host_window failed gle={:?}, falling back to HWND_MESSAGE",
            GetLastError()
        ));
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("TsfHostTextStoreWindow"),
            w!("TsfHostTextStoreWindow"),
            WINDOW_STYLE(0),
            0,
            0,
            800,
            600,
            HWND_MESSAGE,
            None,
            hinstance,
            None,
        )
        .unwrap_or_default();
        tsf_step(format!("[tsf-text] create_host_window hwnd={:?}", hwnd));
        hwnd
    }
}

/// Shared buffer for reading composition in `get_context`.
pub struct TextStoreInner {
    pub buf: Mutex<Vec<u16>>,
    pub sel_start: Mutex<i32>,
    pub sel_end: Mutex<i32>,
    composition_active: Mutex<bool>,
    sink: Mutex<Option<ITextStoreACPSink>>,
    sink_mask: Mutex<u32>,
    requested_attrs: Mutex<VecDeque<(GUID, bool)>>,
    hwnd: HWND,
}

impl Default for TextStoreInner {
    fn default() -> Self {
        Self {
            buf: Mutex::new(Vec::new()),
            sel_start: Mutex::new(0),
            sel_end: Mutex::new(0),
            composition_active: Mutex::new(false),
            sink: Mutex::new(None),
            sink_mask: Mutex::new(0),
            requested_attrs: Mutex::new(VecDeque::new()),
            hwnd: create_host_window(),
        }
    }
}

impl TextStoreInner {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn snapshot_utf8(&self) -> String {
        let b = self.buf.lock().unwrap();
        String::from_utf16_lossy(&b)
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    fn screen_rect(&self) -> RECT {
        unsafe {
            let mut rect = RECT::default();
            if self.hwnd.is_invalid() {
                return rect;
            }
            let _ = GetWindowRect(self.hwnd, &mut rect);
            rect
        }
    }
}

impl Drop for TextStoreInner {
    fn drop(&mut self) {
        unsafe {
            if !self.hwnd.is_invalid() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

#[implement(ITextStoreACP, ITfContextOwnerCompositionSink)]
pub struct TextStoreAcp {
    pub inner: Arc<TextStoreInner>,
}

impl TextStoreAcp {
    pub fn with_inner(inner: Arc<TextStoreInner>) -> Self {
        Self { inner }
    }

    fn current_attr_range(&self) -> (i32, i32) {
        let sel_start = *self.inner.sel_start.lock().unwrap();
        let sel_end = *self.inner.sel_end.lock().unwrap();
        let buf_len = self.inner.buf.lock().unwrap().len() as i32;
        let start = sel_start.clamp(0, buf_len);
        let end = sel_end.clamp(start, buf_len);
        (start, end)
    }

    fn notify_composition_attr_change(&self, start: i32, end: i32) {
        let sink = self.inner.sink.lock().unwrap().clone();
        if let Some(s) = sink.as_ref() {
            unsafe {
                let attrs = [GUID_PROP_COMPOSING];
                let _ = s.OnAttrsChange(start, end, &attrs);
            }
            tsf_step(format!(
                "[tsf-text] OnAttrsChange GUID_PROP_COMPOSING range=[{}..{}]",
                start, end
            ));
        }
    }

    fn notify_text_change(&self, change: &TS_TEXTCHANGE, is_correction: bool) {
        let sink = self.inner.sink.lock().unwrap().clone();
        if let Some(s) = sink.as_ref() {
            unsafe {
                let flags = if is_correction {
                    TS_ST_CORRECTION
                } else {
                    TS_ST_NONE
                };
                let _ = s.OnTextChange(flags, change as *const _);
                let _ = s.OnSelectionChange();
                let _ = s.OnLayoutChange(TS_LC_CHANGE, 0);
            }
        }

        if *self.inner.composition_active.lock().unwrap() {
            self.notify_composition_attr_change(change.acpStart, change.acpNewEnd);
        }
    }

    fn notify_edit_transaction(&self, start: bool) {
        let sink = self.inner.sink.lock().unwrap().clone();
        if let Some(s) = sink.as_ref() {
            unsafe {
                if start {
                    tsf_step("[tsf-text] OnStartEditTransaction");
                    let _ = s.OnStartEditTransaction();
                } else {
                    tsf_step("[tsf-text] OnEndEditTransaction");
                    let _ = s.OnEndEditTransaction();
                }
            }
        }
    }

    fn insert_text_range(
        &self,
        acpstart: i32,
        acpend: i32,
        pchtext: &PCWSTR,
        cch: u32,
    ) -> windows::core::Result<TS_TEXTCHANGE> {
        let mut buf = self.inner.buf.lock().unwrap();
        let len = buf.len() as i32;
        let composition_active = *self.inner.composition_active.lock().unwrap();
        let replace_entire_composition =
            composition_active && len > 0 && cch > 0 && acpstart == 0 && acpend == 0;
        let start = if replace_entire_composition {
            0usize
        } else {
            acpstart.max(0).min(len) as usize
        };
        let end = if replace_entire_composition {
            len as usize
        } else {
            acpend.max(0).min(len) as usize
        };
        let text: Vec<u16> = if cch == 0 || pchtext.is_null() {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(pchtext.0, cch as usize).to_vec() }
        };
        let old_end = end as i32;
        buf.splice(start..end, text.iter().copied());
        let change = TS_TEXTCHANGE {
            acpStart: start as i32,
            acpOldEnd: old_end,
            acpNewEnd: start as i32 + text.len() as i32,
        };
        let caret = change.acpNewEnd;
        drop(buf);
        *self.inner.sel_start.lock().unwrap() = caret;
        *self.inner.sel_end.lock().unwrap() = caret;
        Ok(change)
    }

    fn query_insert_range(&self, acpstart: i32, acpend: i32, cch: u32) -> TS_TEXTCHANGE {
        let len = self.inner.buf.lock().unwrap().len() as i32;
        let start = acpstart.clamp(0, len);
        let end = acpend.clamp(start, len);
        TS_TEXTCHANGE {
            acpStart: start,
            acpOldEnd: end,
            acpNewEnd: start + cch as i32,
        }
    }

    fn supported_attr_value(&self, attr: &GUID, want_value: bool) -> Option<VARIANT> {
        if !want_value {
            return Some(VARIANT::default());
        }

        if *attr == GUID_PROP_COMPOSING {
            let active = *self.inner.composition_active.lock().unwrap();
            Some(VARIANT::from(if active { 1i32 } else { 0i32 }))
        } else if *attr == GUID_PROP_TEXTOWNER {
            Some(VARIANT::from(0i32))
        } else if *attr == GUID_PROP_LANGID {
            Some(VARIANT::from(0x0804i32))
        } else if *attr == GUID_PROP_ATTRIBUTE || *attr == GUID_PROP_READING {
            Some(VARIANT::from(0i32))
        } else {
            None
        }
    }
}

impl ITextStoreACP_Impl for TextStoreAcp {
    fn AdviseSink(
        &self,
        riid: *const GUID,
        punk: Option<&windows::core::IUnknown>,
        dwmask: u32,
    ) -> windows::core::Result<()> {
        if riid.is_null() || punk.is_none() {
            return Err(E_NOTIMPL.into());
        }
        unsafe {
            if *riid != <ITextStoreACPSink as windows::core::Interface>::IID {
                return Err(E_NOTIMPL.into());
            }
        }
        let u = punk.unwrap();
        let sink: ITextStoreACPSink = u.cast().map_err(|_| E_NOTIMPL)?;
        tsf_step(format!(
            "[tsf-text] AdviseSink mask=0x{:X} sink={:p}",
            dwmask,
            sink.as_raw()
        ));
        *self.inner.sink.lock().unwrap() = Some(sink);
        *self.inner.sink_mask.lock().unwrap() = dwmask;
        Ok(())
    }

    fn UnadviseSink(&self, punk: Option<&windows::core::IUnknown>) -> windows::core::Result<()> {
        if let Some(p) = punk {
            let cur = self.inner.sink.lock().unwrap().take();
            if let Some(s) = cur {
                if s.as_raw() == p.as_raw() {
                    return Ok(());
                }
            }
        }
        *self.inner.sink.lock().unwrap() = None;
        Ok(())
    }

    fn RequestLock(&self, dwlockflags: u32) -> windows::core::Result<windows::core::HRESULT> {
        tsf_step(format!("[tsf-text] RequestLock flags=0x{:X}", dwlockflags));
        let sink = self.inner.sink.lock().unwrap().clone();
        if let Some(s) = sink.as_ref() {
            unsafe {
                self.notify_edit_transaction(true);
                let _ = s.OnLockGranted(TEXT_STORE_LOCK_FLAGS(dwlockflags));
                self.notify_edit_transaction(false);
            }
        }
        Ok(windows::core::HRESULT(0))
    }

    fn GetStatus(&self) -> windows::core::Result<TS_STATUS> {
        Ok(TS_STATUS {
            dwDynamicFlags: 0,
            dwStaticFlags: TS_SS_NOHIDDENTEXT,
        })
    }

    fn QueryInsert(
        &self,
        acpteststart: i32,
        acptestend: i32,
        cch: u32,
        pacpresultstart: *mut i32,
        pacpresultend: *mut i32,
    ) -> windows::core::Result<()> {
        unsafe {
            if !pacpresultstart.is_null() {
                *pacpresultstart = acpteststart;
            }
            if !pacpresultend.is_null() {
                *pacpresultend = acptestend + cch as i32;
            }
        }
        Ok(())
    }

    fn GetSelection(
        &self,
        ulindex: u32,
        ulcount: u32,
        pselection: *mut TS_SELECTION_ACP,
        pcfetched: *mut u32,
    ) -> windows::core::Result<()> {
        if ulindex != 0 {
            return Err(E_NOTIMPL.into());
        }
        let start = *self.inner.sel_start.lock().unwrap();
        let end = *self.inner.sel_end.lock().unwrap();
        unsafe {
            if !pselection.is_null() && ulcount >= 1 {
                (*pselection).acpStart = start;
                (*pselection).acpEnd = end;
                (*pselection).style = TS_SELECTIONSTYLE {
                    ase: TS_AE_NONE,
                    fInterimChar: false.into(),
                };
            }
            if !pcfetched.is_null() {
                *pcfetched = 1;
            }
        }
        tsf_step(format!(
            "[tsf-text] GetSelection index={} count={} -> [{}..{}]",
            ulindex, ulcount, start, end
        ));
        Ok(())
    }

    fn SetSelection(
        &self,
        ulcount: u32,
        pselection: *const TS_SELECTION_ACP,
    ) -> windows::core::Result<()> {
        if ulcount == 0 || pselection.is_null() {
            return Ok(());
        }
        unsafe {
            let sel = &*pselection;
            *self.inner.sel_start.lock().unwrap() = sel.acpStart;
            *self.inner.sel_end.lock().unwrap() = sel.acpEnd;
            tsf_step(format!(
                "[tsf-text] SetSelection start={} end={}",
                sel.acpStart, sel.acpEnd
            ));
        }
        let sink = self.inner.sink.lock().unwrap().clone();
        if let Some(s) = sink.as_ref() {
            unsafe {
                let _ = s.OnSelectionChange();
                let _ = s.OnLayoutChange(TS_LC_CHANGE, 0);
            }
        }
        Ok(())
    }

    fn GetText(
        &self,
        acpstart: i32,
        acpend: i32,
        pchplain: windows::core::PWSTR,
        cchplainreq: u32,
        pcchplainret: *mut u32,
        prgruninfo: *mut TS_RUNINFO,
        cruninforeq: u32,
        pcruninforet: *mut u32,
        pacpnext: *mut i32,
    ) -> windows::core::Result<()> {
        let buf = self.inner.buf.lock().unwrap();
        let len = buf.len() as i32;
        let end = if acpend < 0 { len } else { acpend.min(len) };
        let start = acpstart.max(0).min(len);
        let slice = &buf[start as usize..end as usize];
        let need = slice.len().min(cchplainreq.saturating_sub(1) as usize);
        unsafe {
            if !pchplain.is_null() && need > 0 {
                std::ptr::copy_nonoverlapping(slice.as_ptr(), pchplain.0, need);
                pchplain.0.add(need).write(0);
            }
            if !pcchplainret.is_null() {
                *pcchplainret = need as u32;
            }
            if !pcruninforet.is_null() {
                *pcruninforet = 0;
            }
            if !prgruninfo.is_null() && cruninforeq > 0 {
                // no run info
            }
            if !pacpnext.is_null() {
                *pacpnext = end;
            }
        }
        tsf_step(format!(
            "[tsf-text] GetText [{}..{}] -> '{}'",
            start,
            end,
            String::from_utf16_lossy(slice)
        ));
        Ok(())
    }

    fn SetText(
        &self,
        dwflags: u32,
        acpstart: i32,
        acpend: i32,
        pchtext: &PCWSTR,
        cch: u32,
    ) -> windows::core::Result<TS_TEXTCHANGE> {
        let change = self.insert_text_range(acpstart, acpend, pchtext, cch)?;
        self.notify_text_change(&change, (dwflags & 0x1) != 0);
        tsf_step(format!(
            "[tsf-text] SetText flags=0x{:X} range=[{}, {}) chars={} composition_active={} -> '{}'",
            dwflags,
            acpstart,
            acpend,
            cch,
            *self.inner.composition_active.lock().unwrap(),
            self.inner.snapshot_utf8()
        ));
        Ok(change)
    }

    fn GetFormattedText(&self, _acpstart: i32, _acpend: i32) -> windows::core::Result<IDataObject> {
        Err(E_NOTIMPL.into())
    }

    fn GetEmbedded(
        &self,
        _acppos: i32,
        _rguidservice: *const GUID,
        _riid: *const GUID,
    ) -> windows::core::Result<windows::core::IUnknown> {
        Err(E_NOTIMPL.into())
    }

    fn QueryInsertEmbedded(
        &self,
        _pguidservice: *const GUID,
        _pformatetc: *const FORMATETC,
    ) -> windows::core::Result<BOOL> {
        Ok(false.into())
    }

    fn InsertEmbedded(
        &self,
        _dwflags: u32,
        _acpstart: i32,
        _acpend: i32,
        _pdataobject: Option<&IDataObject>,
    ) -> windows::core::Result<TS_TEXTCHANGE> {
        Err(E_NOTIMPL.into())
    }

    fn InsertTextAtSelection(
        &self,
        dwflags: u32,
        pchtext: &PCWSTR,
        cch: u32,
        pacpstart: *mut i32,
        pacpend: *mut i32,
        pchange: *mut TS_TEXTCHANGE,
    ) -> windows::core::Result<()> {
        let sel_start = *self.inner.sel_start.lock().unwrap();
        let sel_end = *self.inner.sel_end.lock().unwrap();
        let is_query_only = (dwflags & TS_IAS_QUERYONLY) != 0;
        let is_noquery = (dwflags & TS_IAS_NOQUERY) != 0;
        let ch = if is_query_only {
            self.query_insert_range(sel_start, sel_end, cch)
        } else {
            self.insert_text_range(sel_start, sel_end, pchtext, cch)?
        };
        if is_query_only && *self.inner.composition_active.lock().unwrap() {
            self.notify_composition_attr_change(ch.acpStart, ch.acpNewEnd);
        }
        if !is_query_only {
            self.notify_text_change(&ch, false);
        }
        tsf_step(format!(
            "[tsf-text] InsertTextAtSelection flags=0x{:X} sel=[{}, {}) chars={} query_only={} noquery={} -> '{}'",
            dwflags,
            sel_start,
            sel_end,
            cch,
            is_query_only,
            is_noquery,
            self.inner.snapshot_utf8()
        ));
        unsafe {
            if !is_noquery && !pacpstart.is_null() {
                *pacpstart = sel_start;
            }
            if !is_noquery && !pacpend.is_null() {
                *pacpend = ch.acpNewEnd;
            }
            if !pchange.is_null() {
                *pchange = ch;
            }
        }
        Ok(())
    }

    fn InsertEmbeddedAtSelection(
        &self,
        _dwflags: u32,
        _pdataobject: Option<&IDataObject>,
        _pacpstart: *mut i32,
        _pacpend: *mut i32,
        _pchange: *mut TS_TEXTCHANGE,
    ) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn RequestSupportedAttrs(
        &self,
        dwflags: u32,
        cfilterattrs: u32,
        pafilterattrs: *const GUID,
    ) -> windows::core::Result<()> {
        let want_value = (dwflags & TS_ATTR_FIND_WANT_VALUE) != 0;
        let mut requested = self.inner.requested_attrs.lock().unwrap();
        requested.clear();
        if pafilterattrs.is_null() || cfilterattrs == 0 {
            tsf_step(format!(
                "[tsf-text] RequestSupportedAttrs flags=0x{:X} count={} (no filter attrs)",
                dwflags, cfilterattrs
            ));
            return Ok(());
        }
        for i in 0..cfilterattrs as usize {
            unsafe {
                let attr = *pafilterattrs.add(i);
                requested.push_back((attr, want_value));
                tsf_step(format!(
                    "[tsf-text] RequestSupportedAttrs flags=0x{:X} attr[{}]={:?} want_value={}",
                    dwflags, i, attr, want_value
                ));
            }
        }
        Ok(())
    }

    fn RequestAttrsAtPosition(
        &self,
        acppos: i32,
        cfilterattrs: u32,
        pafilterattrs: *const GUID,
        dwflags: u32,
    ) -> windows::core::Result<()> {
        self.RequestSupportedAttrs(dwflags, cfilterattrs, pafilterattrs)?;
        tsf_step(format!(
            "[tsf-text] RequestAttrsAtPosition pos={} flags=0x{:X} count={}",
            acppos, dwflags, cfilterattrs
        ));
        Ok(())
    }

    fn RequestAttrsTransitioningAtPosition(
        &self,
        acppos: i32,
        cfilterattrs: u32,
        pafilterattrs: *const GUID,
        dwflags: u32,
    ) -> windows::core::Result<()> {
        self.RequestSupportedAttrs(dwflags, cfilterattrs, pafilterattrs)?;
        tsf_step(format!(
            "[tsf-text] RequestAttrsTransitioningAtPosition pos={} flags=0x{:X} count={}",
            acppos, dwflags, cfilterattrs
        ));
        Ok(())
    }

    fn FindNextAttrTransition(
        &self,
        acpstart: i32,
        acphalt: i32,
        _cfilterattrs: u32,
        _pafilterattrs: *const GUID,
        _dwflags: u32,
        pacpnext: *mut i32,
        pffound: *mut BOOL,
        plfoundoffset: *mut i32,
    ) -> windows::core::Result<()> {
        let next = if acphalt >= 0 {
            acphalt
        } else {
            self.GetEndACP()?
        };
        unsafe {
            if !pacpnext.is_null() {
                *pacpnext = next;
            }
            if !pffound.is_null() {
                *pffound = false.into();
            }
            if !plfoundoffset.is_null() {
                *plfoundoffset = (next - acpstart).max(0);
            }
        }
        tsf_step(format!(
            "[tsf-text] FindNextAttrTransition start={} halt={} -> next={} found=false",
            acpstart, acphalt, next
        ));
        Ok(())
    }

    fn RetrieveRequestedAttrs(
        &self,
        ulcount: u32,
        paattrvals: *mut TS_ATTRVAL,
        pcfetched: *mut u32,
    ) -> windows::core::Result<()> {
        let mut requested = self.inner.requested_attrs.lock().unwrap();
        let mut fetched = 0u32;
        while fetched < ulcount && !requested.is_empty() {
            let (attr, want_value) = requested.pop_front().unwrap();
            let Some(value) = self.supported_attr_value(&attr, want_value) else {
                continue;
            };
            unsafe {
                if !paattrvals.is_null() {
                    let slot = paattrvals.add(fetched as usize);
                    (*slot).idAttr = attr;
                    (*slot).dwOverlapId = 0;
                    (*slot).varValue = ManuallyDrop::new(value);
                }
            }
            fetched += 1;
        }
        unsafe {
            if !pcfetched.is_null() {
                *pcfetched = fetched;
            }
        }
        tsf_step(format!(
            "[tsf-text] RetrieveRequestedAttrs count={} fetched={}",
            ulcount, fetched
        ));
        Ok(())
    }

    fn GetEndACP(&self) -> windows::core::Result<i32> {
        Ok(self.inner.buf.lock().unwrap().len() as i32)
    }

    fn GetActiveView(&self) -> windows::core::Result<u32> {
        Ok(0)
    }

    fn GetACPFromPoint(
        &self,
        _vcview: u32,
        ptscreen: *const windows::Win32::Foundation::POINT,
        _dwflags: u32,
    ) -> windows::core::Result<i32> {
        if ptscreen.is_null() {
            return Ok(*self.inner.sel_end.lock().unwrap());
        }

        let rect = self.inner.screen_rect();
        let line_top = rect.top + 8;
        let line_bottom = line_top + 24;
        let line_left = rect.left + 8;
        let content_right = rect.right.max(line_left + 1);

        unsafe {
            let pt = *ptscreen;
            if pt.y < line_top || pt.y >= line_bottom || pt.x < line_left {
                tsf_step(format!(
                    "[tsf-text] GetACPFromPoint outside pt=({}, {}) -> caret={}",
                    pt.x,
                    pt.y,
                    *self.inner.sel_end.lock().unwrap()
                ));
                return Ok(*self.inner.sel_end.lock().unwrap());
            }

            let clamped_x = pt.x.min(content_right);
            let acp = ((clamped_x - line_left) / 9).max(0);
            let len = self.inner.buf.lock().unwrap().len() as i32;
            let result = acp.min(len);
            tsf_step(format!(
                "[tsf-text] GetACPFromPoint pt=({}, {}) -> acp={} len={}",
                pt.x, pt.y, result, len
            ));
            Ok(result)
        }
    }

    fn GetTextExt(
        &self,
        _vcview: u32,
        acpstart: i32,
        acpend: i32,
        prc: *mut RECT,
        pfclipped: *mut BOOL,
    ) -> windows::core::Result<()> {
        let len = self.inner.buf.lock().unwrap().len() as i32;
        let start = acpstart.clamp(0, len);
        let end = acpend.clamp(start, len);
        let width = ((end - start).max(1) * 9).min(800);
        let screen_rect = self.inner.screen_rect();
        unsafe {
            if !prc.is_null() {
                let mut rect = screen_rect;
                rect.left += 8 + start * 9;
                rect.top += 8;
                rect.right = rect.left + width;
                rect.bottom = rect.top + 24;
                *prc = rect;
            }
            if !pfclipped.is_null() {
                *pfclipped = false.into();
            }
        }
        tsf_step(format!(
            "[tsf-text] GetTextExt range=[{}..{}] screen_rect=({}, {}, {}, {})",
            start, end, screen_rect.left, screen_rect.top, screen_rect.right, screen_rect.bottom
        ));
        Ok(())
    }

    fn GetScreenExt(&self, _vcview: u32) -> windows::core::Result<RECT> {
        let rect = self.inner.screen_rect();
        tsf_step(format!(
            "[tsf-text] GetScreenExt -> ({}, {}, {}, {})",
            rect.left, rect.top, rect.right, rect.bottom
        ));
        Ok(rect)
    }

    fn GetWnd(&self, _vcview: u32) -> windows::core::Result<HWND> {
        let hwnd = self.inner.hwnd();
        tsf_step(format!("[tsf-text] GetWnd -> {:?}", hwnd));
        Ok(hwnd)
    }
}

impl ITfContextOwnerCompositionSink_Impl for TextStoreAcp {
    fn OnStartComposition(
        &self,
        _pcomposition: Option<&ITfCompositionView>,
    ) -> windows::core::Result<BOOL> {
        *self.inner.composition_active.lock().unwrap() = true;
        let (start, end) = self.current_attr_range();
        self.notify_composition_attr_change(start, end);
        tsf_step(format!(
            "[tsf-text] OnStartComposition text='{}'",
            self.inner.snapshot_utf8()
        ));
        Ok(true.into())
    }

    fn OnUpdateComposition(
        &self,
        _pcomposition: Option<&ITfCompositionView>,
        _prangenew: Option<&ITfRange>,
    ) -> windows::core::Result<()> {
        let (start, end) = self.current_attr_range();
        self.notify_composition_attr_change(start, end);
        tsf_step(format!(
            "[tsf-text] OnUpdateComposition text='{}'",
            self.inner.snapshot_utf8()
        ));
        Ok(())
    }

    fn OnEndComposition(
        &self,
        _pcomposition: Option<&ITfCompositionView>,
    ) -> windows::core::Result<()> {
        *self.inner.composition_active.lock().unwrap() = false;
        let (start, end) = self.current_attr_range();
        self.notify_composition_attr_change(start, end);
        tsf_step(format!(
            "[tsf-text] OnEndComposition text='{}'",
            self.inner.snapshot_utf8()
        ));
        Ok(())
    }
}

impl ITextStoreACP_Impl for TextStoreAcp_Impl {
    fn AdviseSink(
        &self,
        riid: *const GUID,
        punk: Option<&windows::core::IUnknown>,
        dwmask: u32,
    ) -> windows::core::Result<()> {
        ITextStoreACP_Impl::AdviseSink(&self.this, riid, punk, dwmask)
    }

    fn UnadviseSink(&self, punk: Option<&windows::core::IUnknown>) -> windows::core::Result<()> {
        ITextStoreACP_Impl::UnadviseSink(&self.this, punk)
    }

    fn RequestLock(&self, dwlockflags: u32) -> windows::core::Result<windows::core::HRESULT> {
        ITextStoreACP_Impl::RequestLock(&self.this, dwlockflags)
    }

    fn GetStatus(&self) -> windows::core::Result<TS_STATUS> {
        ITextStoreACP_Impl::GetStatus(&self.this)
    }

    fn QueryInsert(
        &self,
        acpteststart: i32,
        acptestend: i32,
        cch: u32,
        pacpresultstart: *mut i32,
        pacpresultend: *mut i32,
    ) -> windows::core::Result<()> {
        ITextStoreACP_Impl::QueryInsert(
            &self.this,
            acpteststart,
            acptestend,
            cch,
            pacpresultstart,
            pacpresultend,
        )
    }

    fn GetSelection(
        &self,
        ulindex: u32,
        ulcount: u32,
        pselection: *mut TS_SELECTION_ACP,
        pcfetched: *mut u32,
    ) -> windows::core::Result<()> {
        ITextStoreACP_Impl::GetSelection(&self.this, ulindex, ulcount, pselection, pcfetched)
    }

    fn SetSelection(
        &self,
        ulcount: u32,
        pselection: *const TS_SELECTION_ACP,
    ) -> windows::core::Result<()> {
        ITextStoreACP_Impl::SetSelection(&self.this, ulcount, pselection)
    }

    fn GetText(
        &self,
        acpstart: i32,
        acpend: i32,
        pchplain: windows::core::PWSTR,
        cchplainreq: u32,
        pcchplainret: *mut u32,
        prgruninfo: *mut TS_RUNINFO,
        cruninforeq: u32,
        pcruninforet: *mut u32,
        pacpnext: *mut i32,
    ) -> windows::core::Result<()> {
        ITextStoreACP_Impl::GetText(
            &self.this,
            acpstart,
            acpend,
            pchplain,
            cchplainreq,
            pcchplainret,
            prgruninfo,
            cruninforeq,
            pcruninforet,
            pacpnext,
        )
    }

    fn SetText(
        &self,
        dwflags: u32,
        acpstart: i32,
        acpend: i32,
        pchtext: &PCWSTR,
        cch: u32,
    ) -> windows::core::Result<TS_TEXTCHANGE> {
        ITextStoreACP_Impl::SetText(&self.this, dwflags, acpstart, acpend, pchtext, cch)
    }

    fn GetFormattedText(&self, acpstart: i32, acpend: i32) -> windows::core::Result<IDataObject> {
        ITextStoreACP_Impl::GetFormattedText(&self.this, acpstart, acpend)
    }

    fn GetEmbedded(
        &self,
        acppos: i32,
        rguidservice: *const GUID,
        riid: *const GUID,
    ) -> windows::core::Result<windows::core::IUnknown> {
        ITextStoreACP_Impl::GetEmbedded(&self.this, acppos, rguidservice, riid)
    }

    fn QueryInsertEmbedded(
        &self,
        pguidservice: *const GUID,
        pformatetc: *const FORMATETC,
    ) -> windows::core::Result<BOOL> {
        ITextStoreACP_Impl::QueryInsertEmbedded(&self.this, pguidservice, pformatetc)
    }

    fn InsertEmbedded(
        &self,
        dwflags: u32,
        acpstart: i32,
        acpend: i32,
        pdataobject: Option<&IDataObject>,
    ) -> windows::core::Result<TS_TEXTCHANGE> {
        ITextStoreACP_Impl::InsertEmbedded(&self.this, dwflags, acpstart, acpend, pdataobject)
    }

    fn InsertTextAtSelection(
        &self,
        dwflags: u32,
        pchtext: &PCWSTR,
        cch: u32,
        pacpstart: *mut i32,
        pacpend: *mut i32,
        pchange: *mut TS_TEXTCHANGE,
    ) -> windows::core::Result<()> {
        ITextStoreACP_Impl::InsertTextAtSelection(
            &self.this, dwflags, pchtext, cch, pacpstart, pacpend, pchange,
        )
    }

    fn InsertEmbeddedAtSelection(
        &self,
        dwflags: u32,
        pdataobject: Option<&IDataObject>,
        pacpstart: *mut i32,
        pacpend: *mut i32,
        pchange: *mut TS_TEXTCHANGE,
    ) -> windows::core::Result<()> {
        ITextStoreACP_Impl::InsertEmbeddedAtSelection(
            &self.this,
            dwflags,
            pdataobject,
            pacpstart,
            pacpend,
            pchange,
        )
    }

    fn RequestSupportedAttrs(
        &self,
        dwflags: u32,
        cfilterattrs: u32,
        pafilterattrs: *const GUID,
    ) -> windows::core::Result<()> {
        ITextStoreACP_Impl::RequestSupportedAttrs(&self.this, dwflags, cfilterattrs, pafilterattrs)
    }

    fn RequestAttrsAtPosition(
        &self,
        acppos: i32,
        cfilterattrs: u32,
        pafilterattrs: *const GUID,
        dwflags: u32,
    ) -> windows::core::Result<()> {
        ITextStoreACP_Impl::RequestAttrsAtPosition(
            &self.this,
            acppos,
            cfilterattrs,
            pafilterattrs,
            dwflags,
        )
    }

    fn RequestAttrsTransitioningAtPosition(
        &self,
        acppos: i32,
        cfilterattrs: u32,
        pafilterattrs: *const GUID,
        dwflags: u32,
    ) -> windows::core::Result<()> {
        ITextStoreACP_Impl::RequestAttrsTransitioningAtPosition(
            &self.this,
            acppos,
            cfilterattrs,
            pafilterattrs,
            dwflags,
        )
    }

    fn FindNextAttrTransition(
        &self,
        acpstart: i32,
        acphalt: i32,
        cfilterattrs: u32,
        pafilterattrs: *const GUID,
        dwflags: u32,
        pacpnext: *mut i32,
        pffound: *mut BOOL,
        plfoundoffset: *mut i32,
    ) -> windows::core::Result<()> {
        ITextStoreACP_Impl::FindNextAttrTransition(
            &self.this,
            acpstart,
            acphalt,
            cfilterattrs,
            pafilterattrs,
            dwflags,
            pacpnext,
            pffound,
            plfoundoffset,
        )
    }

    fn RetrieveRequestedAttrs(
        &self,
        ulcount: u32,
        paattrvals: *mut TS_ATTRVAL,
        pcfetched: *mut u32,
    ) -> windows::core::Result<()> {
        ITextStoreACP_Impl::RetrieveRequestedAttrs(&self.this, ulcount, paattrvals, pcfetched)
    }

    fn GetEndACP(&self) -> windows::core::Result<i32> {
        ITextStoreACP_Impl::GetEndACP(&self.this)
    }

    fn GetActiveView(&self) -> windows::core::Result<u32> {
        ITextStoreACP_Impl::GetActiveView(&self.this)
    }

    fn GetACPFromPoint(
        &self,
        vcview: u32,
        ptscreen: *const windows::Win32::Foundation::POINT,
        dwflags: u32,
    ) -> windows::core::Result<i32> {
        ITextStoreACP_Impl::GetACPFromPoint(&self.this, vcview, ptscreen, dwflags)
    }

    fn GetTextExt(
        &self,
        vcview: u32,
        acpstart: i32,
        acpend: i32,
        prc: *mut RECT,
        pfclipped: *mut BOOL,
    ) -> windows::core::Result<()> {
        ITextStoreACP_Impl::GetTextExt(&self.this, vcview, acpstart, acpend, prc, pfclipped)
    }

    fn GetScreenExt(&self, vcview: u32) -> windows::core::Result<RECT> {
        ITextStoreACP_Impl::GetScreenExt(&self.this, vcview)
    }

    fn GetWnd(&self, vcview: u32) -> windows::core::Result<HWND> {
        ITextStoreACP_Impl::GetWnd(&self.this, vcview)
    }
}

impl ITfContextOwnerCompositionSink_Impl for TextStoreAcp_Impl {
    fn OnStartComposition(
        &self,
        pcomposition: Option<&ITfCompositionView>,
    ) -> windows::core::Result<BOOL> {
        ITfContextOwnerCompositionSink_Impl::OnStartComposition(&self.this, pcomposition)
    }

    fn OnUpdateComposition(
        &self,
        pcomposition: Option<&ITfCompositionView>,
        prangenew: Option<&ITfRange>,
    ) -> windows::core::Result<()> {
        ITfContextOwnerCompositionSink_Impl::OnUpdateComposition(
            &self.this,
            pcomposition,
            prangenew,
        )
    }

    fn OnEndComposition(
        &self,
        pcomposition: Option<&ITfCompositionView>,
    ) -> windows::core::Result<()> {
        ITfContextOwnerCompositionSink_Impl::OnEndComposition(&self.this, pcomposition)
    }
}
