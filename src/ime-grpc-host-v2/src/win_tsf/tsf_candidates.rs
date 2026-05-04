//! Read TSF candidate list via `ITfUIElementMgr` → `ITfCandidateListUIElement` (same plane as on-screen UI).

use windows::core::Interface;
use windows::Win32::UI::TextServices::{
    ITfCandidateListUIElement, ITfCandidateListUIElementBehavior, ITfDocumentMgr, ITfThreadMgr,
    ITfUIElement, ITfUIElementMgr,
};

use crate::proto::rime_service_v2::{CandidateProto, MenuProto};

use super::diag::{tsf_step, tsf_warn};

/// Prefer a candidate list UI attached to `doc_mgr` when possible; otherwise the first non-empty list.
pub unsafe fn find_candidate_list_ui_element(
    thread_mgr: &ITfThreadMgr,
    doc_mgr: Option<&ITfDocumentMgr>,
) -> Option<ITfCandidateListUIElement> {
    let uiem: ITfUIElementMgr = thread_mgr.cast().ok()?;
    let enum_ = uiem.EnumUIElements().ok()?;
    let _ = enum_.Reset();

    let mut buf = vec![None::<ITfUIElement>; 16];
    let mut best_matching: Option<ITfCandidateListUIElement> = None;
    let mut best_any: Option<ITfCandidateListUIElement> = None;

    loop {
        let mut fetched = 0u32;
        if enum_
            .Next(&mut buf, std::ptr::addr_of_mut!(fetched))
            .is_err()
        {
            break;
        }
        if fetched == 0 {
            break;
        }

        for i in 0..(fetched as usize).min(buf.len()) {
            let Some(ref el) = buf[i] else {
                continue;
            };
            let Ok(cand_ui) = el.cast::<ITfCandidateListUIElement>() else {
                continue;
            };
            let Ok(n) = cand_ui.GetCount() else {
                continue;
            };
            if n == 0 {
                continue;
            }
            tsf_step(format!(
                "[tsf-ui] candidate ui element count={} doc_match={}",
                n,
                doc_mgr
                    .and_then(|want| cand_ui
                        .GetDocumentMgr()
                        .ok()
                        .map(|got| got.as_raw() == want.as_raw()))
                    .unwrap_or(false)
            ));

            if let Some(want) = doc_mgr {
                if let Ok(got) = cand_ui.GetDocumentMgr() {
                    if got.as_raw() == want.as_raw() {
                        best_matching = Some(cand_ui);
                        break;
                    }
                }
            }
            if best_any.is_none() {
                best_any = Some(cand_ui);
            }
        }

        if best_matching.is_some() {
            break;
        }
    }

    best_matching.or(best_any)
}

pub unsafe fn highlighted_candidate_text(
    thread_mgr: &ITfThreadMgr,
    doc_mgr: Option<&ITfDocumentMgr>,
) -> Option<String> {
    let cand_ui = find_candidate_list_ui_element(thread_mgr, doc_mgr)?;
    let count = cand_ui.GetCount().ok()?;
    if count == 0 {
        tsf_warn("[tsf-ui] candidate ui element exists but count=0");
        return None;
    }
    let selection = cand_ui.GetSelection().unwrap_or(0).min(count - 1);
    cand_ui.GetString(selection).ok().map(|s| s.to_string())
}

pub unsafe fn candidate_text_at_index(
    thread_mgr: &ITfThreadMgr,
    doc_mgr: Option<&ITfDocumentMgr>,
    index: u32,
) -> Option<String> {
    let cand_ui = find_candidate_list_ui_element(thread_mgr, doc_mgr)?;
    let count = cand_ui.GetCount().ok()?;
    if count == 0 || index >= count {
        return None;
    }
    cand_ui.GetString(index).ok().map(|s| s.to_string())
}

pub unsafe fn select_candidate_by_index(
    thread_mgr: &ITfThreadMgr,
    doc_mgr: Option<&ITfDocumentMgr>,
    index: u32,
) -> bool {
    let Some(cand_ui) = find_candidate_list_ui_element(thread_mgr, doc_mgr) else {
        return false;
    };
    let Ok(count) = cand_ui.GetCount() else {
        return false;
    };
    if count == 0 || index >= count {
        return false;
    }
    let Ok(behavior) = cand_ui.cast::<ITfCandidateListUIElementBehavior>() else {
        tsf_warn("[tsf-ui] ITfCandidateListUIElementBehavior unavailable");
        return false;
    };
    if behavior.SetSelection(index).is_err() {
        return false;
    }
    behavior.Finalize().is_ok()
}

/// Prefer a candidate list UI attached to `doc_mgr` when possible; otherwise the first non-empty list.
pub unsafe fn menu_from_ui_element_mgr(
    thread_mgr: &ITfThreadMgr,
    doc_mgr: Option<&ITfDocumentMgr>,
) -> Option<MenuProto> {
    let cand_ui = find_candidate_list_ui_element(thread_mgr, doc_mgr)?;
    let count = cand_ui.GetCount().ok()?;
    if count == 0 {
        tsf_warn("[tsf-ui] candidate ui element exists but count=0");
        return None;
    }
    let selection = cand_ui.GetSelection().unwrap_or(0).min(count - 1);
    let current_page = cand_ui.GetCurrentPage().unwrap_or(0);
    let mut candidates = Vec::new();
    for i in 0..count {
        let Ok(s) = cand_ui.GetString(i) else {
            continue;
        };
        candidates.push(CandidateProto {
            text: s.to_string(),
            comment: String::new(),
            quality: 0.0,
        });
    }
    if candidates.is_empty() {
        tsf_warn("[tsf-ui] candidate ui element yielded zero strings");
        return None;
    }

    tsf_step(format!(
        "[tsf-ui] menu_from_ui_element_mgr -> {} candidates, selection={}",
        count, selection
    ));
    Some(MenuProto {
        candidates,
        page_size: 9,
        page_no: current_page as i32,
        is_last_page: false,
        highlighted_candidate_index: selection as i32,
        num_candidates: count as i32,
        select_keys: String::new(),
    })
}
