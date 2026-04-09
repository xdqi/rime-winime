use super::rime_ffi;
use crate::backend::RimeBackend;
use crate::proto::rime_service_v2::{
    CandidateProto, CompositionProto, KeyEvent, MenuProto, RimeContextProto,
};
use std::ffi::{CStr, CString};
use std::ptr;

pub struct NativeRimeBackend {
    api: *const rime_ffi::RimeApi,
    _shared_data_dir: CString,
    _user_data_dir: CString,
    _distribution_name: CString,
    _distribution_code_name: CString,
    _distribution_version: CString,
    _app_name: CString,
}

unsafe impl Send for NativeRimeBackend {}
unsafe impl Sync for NativeRimeBackend {}

impl NativeRimeBackend {
    pub fn new() -> Self {
        unsafe {
            let api = rime_ffi::rime_get_api();

            let shared = CString::new("/usr/share/rime-data").unwrap();
            let user = CString::new("/home/user/.config/ibus/rime").unwrap();
            let name = CString::new("Rime").unwrap();
            let code = CString::new("rime").unwrap();
            let version = CString::new("1.0").unwrap();
            let app = CString::new("rime.grpc.v2").unwrap();

            let mut traits = rime_ffi::RimeTraits {
                data_size: std::mem::size_of::<rime_ffi::RimeTraits>() as libc::c_int
                    - std::mem::size_of::<libc::c_int>() as libc::c_int,
                shared_data_dir: shared.as_ptr(),
                user_data_dir: user.as_ptr(),
                distribution_name: name.as_ptr(),
                distribution_code_name: code.as_ptr(),
                distribution_version: version.as_ptr(),
                app_name: app.as_ptr(),
                modules: ptr::null(),
                min_log_level: 0,
                log_dir: ptr::null(),
                prebuilt_data_dir: ptr::null(),
                staging_dir: ptr::null(),
            };

            ((*api).setup)(&mut traits);
            ((*api).initialize)(&mut traits);

            Self {
                api,
                _shared_data_dir: shared,
                _user_data_dir: user,
                _distribution_name: name,
                _distribution_code_name: code,
                _distribution_version: version,
                _app_name: app,
            }
        }
    }
}

impl Drop for NativeRimeBackend {
    fn drop(&mut self) {
        unsafe {
            ((*self.api).finalize)();
        }
    }
}

#[tonic::async_trait]
impl RimeBackend for NativeRimeBackend {
    async fn open_session(&mut self) -> Option<usize> {
        unsafe {
            let session_id = ((*self.api).create_session)();
            if session_id != 0 {
                let schema_id = CString::new("luna_pinyin").unwrap();
                ((*self.api).select_schema)(session_id, schema_id.as_ptr());
                return Some(session_id as usize);
            }
            None
        }
    }

    async fn destroy_session(&mut self, session_id: usize) {
        unsafe {
            ((*self.api).destroy_session)(session_id as rime_ffi::RimeSessionId);
        }
    }

    async fn process_key(&mut self, session_id: usize, key: &KeyEvent) -> bool {
        unsafe {
            let accepted = ((*self.api).process_key)(
                session_id as rime_ffi::RimeSessionId,
                key.keycode as libc::c_int,
                key.modifier as libc::c_int,
            );
            accepted != 0
        }
    }

    async fn get_context(&mut self, session_id: usize) -> RimeContextProto {
        unsafe {
            let mut ctx: rime_ffi::RimeContext = std::mem::zeroed();
            ctx.data_size = std::mem::size_of::<rime_ffi::RimeContext>() as libc::c_int
                - std::mem::size_of::<libc::c_int>() as libc::c_int;

            if ((*self.api).get_context)(session_id as rime_ffi::RimeSessionId, &mut ctx) != 0 {
                let mut comp_proto = None;
                if !ctx.composition.preedit.is_null() {
                    let preedit = CStr::from_ptr(ctx.composition.preedit)
                        .to_string_lossy()
                        .into_owned();
                    comp_proto = Some(CompositionProto {
                        length: ctx.composition.length,
                        cursor_pos: ctx.composition.cursor_pos,
                        sel_start: ctx.composition.sel_start,
                        sel_end: ctx.composition.sel_end,
                        preedit,
                    });
                }

                let mut menu_proto = None;
                if ctx.menu.num_candidates > 0 {
                    let mut cands = Vec::new();
                    let cand_array = std::slice::from_raw_parts(
                        ctx.menu.candidates,
                        ctx.menu.num_candidates as usize,
                    );
                    for cand in cand_array {
                        let text = if !cand.text.is_null() {
                            CStr::from_ptr(cand.text).to_string_lossy().into_owned()
                        } else {
                            String::new()
                        };

                        let comment = if !cand.comment.is_null() {
                            CStr::from_ptr(cand.comment).to_string_lossy().into_owned()
                        } else {
                            String::new()
                        };

                        cands.push(CandidateProto {
                            text,
                            comment,
                            quality: 0.0,
                        });
                    }

                    menu_proto = Some(MenuProto {
                        candidates: cands,
                        page_size: ctx.menu.page_size,
                        page_no: ctx.menu.page_no,
                        is_last_page: ctx.menu.is_last_page != 0,
                        highlighted_candidate_index: ctx.menu.highlighted_candidate_index,
                        num_candidates: ctx.menu.num_candidates,
                        select_keys: if !ctx.menu.select_keys.is_null() {
                            CStr::from_ptr(ctx.menu.select_keys)
                                .to_string_lossy()
                                .into_owned()
                        } else {
                            String::new()
                        },
                    });
                }

                let commit_text_preview = if !ctx.commit_text_preview.is_null() {
                    CStr::from_ptr(ctx.commit_text_preview)
                        .to_string_lossy()
                        .into_owned()
                } else {
                    String::new()
                };

                let proto = RimeContextProto {
                    composition: comp_proto,
                    menu: menu_proto,
                    commit_text_preview,
                };

                ((*self.api).free_context)(&mut ctx);
                return proto;
            }

            RimeContextProto {
                composition: None,
                menu: None,
                commit_text_preview: String::new(),
            }
        }
    }

    async fn get_commit(&mut self, session_id: usize) -> Option<String> {
        unsafe {
            let mut commit: rime_ffi::RimeCommit = std::mem::zeroed();
            commit.data_size = std::mem::size_of::<rime_ffi::RimeCommit>() as libc::c_int
                - std::mem::size_of::<libc::c_int>() as libc::c_int;

            if ((*self.api).get_commit)(session_id as rime_ffi::RimeSessionId, &mut commit) != 0 {
                let text = if !commit.text.is_null() {
                    CStr::from_ptr(commit.text).to_string_lossy().into_owned()
                } else {
                    String::new()
                };
                ((*self.api).free_commit)(&mut commit);
                return Some(text);
            }
            None
        }
    }

    async fn select_candidate(&mut self, _session_id: usize, _index: usize) -> bool {
        // Fallback or implemented for native later
        false
    }
}
