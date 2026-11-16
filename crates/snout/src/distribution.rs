//! Desktop distribution policy, independent of firmware analysis.

pub const MICROSOFT_STORE: bool = cfg!(feature = "microsoft-store");
pub const STORE_URI: &str = "ms-windows-store://pdp/?productid=9NWM3QW158VZ";
pub const STORE_UPDATE_MESSAGE: &str = "Updates are managed by Microsoft Store.";
pub const PRIVACY_URL: &str = "https://rustypig91.github.io/snout-firmware-explorer/privacy/";

pub const fn application_directory(store: bool) -> &'static str {
    if store {
        "snout-firmware-explorer-store"
    } else {
        "snout-firmware-explorer"
    }
}

/// Reject before spawning workers or accessing the network or executable.
pub fn require_self_updates() -> std::io::Result<()> {
    if MICROSOFT_STORE {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            STORE_UPDATE_MESSAGE,
        ))
    } else {
        Ok(())
    }
}
