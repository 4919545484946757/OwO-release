//! Real Windows display regression: BMP dimensions must match the physical
//! primary display mode, including when the test process is DPI-unaware.
#![cfg(target_os = "windows")]

#[test]
fn screen_capture_covers_the_physical_primary_display() {
    use windows_sys::Win32::Graphics::Gdi::{
        EnumDisplaySettingsW, DEVMODEW, ENUM_CURRENT_SETTINGS,
    };

    let mut mode: DEVMODEW = unsafe { std::mem::zeroed() };
    mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
    let found = unsafe { EnumDisplaySettingsW(std::ptr::null(), ENUM_CURRENT_SETTINGS, &mut mode) };
    assert_ne!(found, 0, "real primary display mode is required");
    let bitmap =
        owo_agent_kernel::platform::capture_screen().expect("real desktop capture must succeed");
    assert!(bitmap.len() >= 54 && &bitmap[..2] == b"BM");
    let width = i32::from_le_bytes(bitmap[18..22].try_into().unwrap());
    let height = i32::from_le_bytes(bitmap[22..26].try_into().unwrap()).unsigned_abs();
    eprintln!(
        "captured={width}x{height}, physical={}x{}",
        mode.dmPelsWidth, mode.dmPelsHeight
    );
    assert_eq!(
        width as u32, mode.dmPelsWidth,
        "capture must not crop the right side"
    );
    assert_eq!(
        height, mode.dmPelsHeight,
        "capture must not crop the bottom"
    );
}
