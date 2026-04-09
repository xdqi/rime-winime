use libc::{c_char, c_int, c_void, uintptr_t};

pub type Bool = c_int;
pub const TRUE: Bool = 1;
pub const FALSE: Bool = 0;

pub type RimeSessionId = uintptr_t;

#[repr(C)]
pub struct RimeTraits {
    pub data_size: c_int,
    pub shared_data_dir: *const c_char,
    pub user_data_dir: *const c_char,
    pub distribution_name: *const c_char,
    pub distribution_code_name: *const c_char,
    pub distribution_version: *const c_char,
    pub app_name: *const c_char,
    pub modules: *const *const c_char,
    pub min_log_level: c_int,
    pub log_dir: *const c_char,
    pub prebuilt_data_dir: *const c_char,
    pub staging_dir: *const c_char,
}

#[repr(C)]
pub struct RimeComposition {
    pub length: c_int,
    pub cursor_pos: c_int,
    pub sel_start: c_int,
    pub sel_end: c_int,
    pub preedit: *mut c_char,
}

#[repr(C)]
pub struct RimeCandidate {
    pub text: *mut c_char,
    pub comment: *mut c_char,
    pub reserved: *mut c_void,
}

#[repr(C)]
pub struct RimeMenu {
    pub page_size: c_int,
    pub page_no: c_int,
    pub is_last_page: Bool,
    pub highlighted_candidate_index: c_int,
    pub num_candidates: c_int,
    pub candidates: *mut RimeCandidate,
    pub select_keys: *mut c_char,
}

#[repr(C)]
pub struct RimeCommit {
    pub data_size: c_int,
    pub text: *mut c_char,
}

#[repr(C)]
pub struct RimeContext {
    pub data_size: c_int,
    pub composition: RimeComposition,
    pub menu: RimeMenu,
    pub commit_text_preview: *mut c_char,
    pub select_labels: *mut *mut c_char,
}

#[repr(C)]
pub struct RimeApi {
    pub data_size: c_int,
    pub setup: extern "C" fn(*mut RimeTraits),
    pub set_notification_handler: extern "C" fn(*mut c_void, *mut c_void),
    pub initialize: extern "C" fn(*mut RimeTraits),
    pub finalize: extern "C" fn(),
    pub start_maintenance: extern "C" fn(Bool) -> Bool,
    pub is_maintenance_mode: extern "C" fn() -> Bool,
    pub join_maintenance_thread: extern "C" fn(),
    pub deployer_initialize: extern "C" fn(*mut RimeTraits),
    pub prebuild: extern "C" fn() -> Bool,
    pub deploy: extern "C" fn() -> Bool,
    pub deploy_schema: extern "C" fn(*const c_char) -> Bool,
    pub deploy_config_file: extern "C" fn(*const c_char, *const c_char) -> Bool,
    pub sync_user_data: extern "C" fn() -> Bool,
    pub create_session: extern "C" fn() -> RimeSessionId,
    pub find_session: extern "C" fn(RimeSessionId) -> Bool,
    pub destroy_session: extern "C" fn(RimeSessionId) -> Bool,
    pub cleanup_stale_sessions: extern "C" fn(),
    pub cleanup_all_sessions: extern "C" fn(),
    pub process_key: extern "C" fn(RimeSessionId, c_int, c_int) -> Bool,
    pub commit_composition: extern "C" fn(RimeSessionId) -> Bool,
    pub clear_composition: extern "C" fn(RimeSessionId),
    pub get_commit: extern "C" fn(RimeSessionId, *mut RimeCommit) -> Bool,
    pub free_commit: extern "C" fn(*mut RimeCommit) -> Bool,
    pub get_context: extern "C" fn(RimeSessionId, *mut RimeContext) -> Bool,
    pub free_context: extern "C" fn(*mut RimeContext) -> Bool,
    pub get_status: extern "C" fn(RimeSessionId, *mut c_void) -> Bool,
    pub free_status: extern "C" fn(*mut c_void) -> Bool,
    pub set_option: extern "C" fn(RimeSessionId, *const c_char, Bool),
    pub get_option: extern "C" fn(RimeSessionId, *const c_char) -> Bool,
    pub set_property: extern "C" fn(RimeSessionId, *const c_char, *const c_char),
    pub get_property:
        extern "C" fn(RimeSessionId, *const c_char, *mut c_char, libc::size_t) -> Bool,
    pub get_schema_list: extern "C" fn(*mut c_void) -> Bool,
    pub free_schema_list: extern "C" fn(*mut c_void),
    pub get_current_schema: extern "C" fn(RimeSessionId, *mut c_char, libc::size_t) -> Bool,
    pub select_schema: extern "C" fn(RimeSessionId, *const c_char) -> Bool,
}

#[link(name = "rime")]
extern "C" {
    pub fn rime_get_api() -> *const RimeApi;
}
