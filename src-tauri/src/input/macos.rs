// macOS accessibility permission handling
// Required for input simulation on macOS

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: *const core_foundation::dictionary::__CFDictionary) -> bool;
}

/// Check if the process has accessibility permission
pub fn has_accessibility_permission() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Request accessibility permission
/// Returns true if permission is already granted, false if user needs to grant it
pub fn request_accessibility_permission() -> bool {
    unsafe {
        // Create options dictionary with kAXTrustedCheckOptionPrompt = true
        // This will show the system prompt if not trusted
        let key = CFString::new("AXTrustedCheckOptionPrompt");
        let value = CFBoolean::true_value();
        let options = CFDictionary::from_CFType_pairs(&[(key, value)]);

        AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef())
    }
}
