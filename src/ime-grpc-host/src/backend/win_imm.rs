#[cfg(windows)]
mod imp;
#[cfg(not(windows))]
mod stub;

#[cfg(windows)]
pub use imp::WinImmBackend;
#[cfg(not(windows))]
pub use stub::WinImmBackend;
