//! Collection-specific visuals over the shared engine chainloader.
#![no_std]
#![no_main]
use psx_chainloader::paint;
use psx_chainloader::runtime::{self, Presentation};
mod loading;
use loading::LoadingScreen;
impl Presentation for LoadingScreen {
    fn setup(&mut self) {
        paint::setup(319, 239);
    }
    fn begin(&mut self) {
        LoadingScreen::begin(self);
    }
    fn update(&mut self, done: u32, total: u32) {
        LoadingScreen::update(self, done, total);
    }
    fn finish(&mut self) {
        LoadingScreen::finish(self);
    }
    fn diagnostic_mode(&mut self) {
        paint::diagnostic_mode();
    }
    fn no_scratchpad(&mut self) {
        paint::diagnostic_mode();
        paint::text(112, 104, 2, "NOSPAD", paint::YELLOW);
    }
}
/// Enter the copied high-RAM blob with a trusted packed executable.
/// # Safety
/// All shared `runtime::run` extent, hardware ownership and stack requirements apply.
#[link_section = ".text.loader_entry"]
#[no_mangle]
pub unsafe extern "C" fn loader_entry(
    exe_lba: u32,
    lba_offset: u32,
    track_base: u32,
    checksum: u32,
) -> ! {
    unsafe {
        runtime::run(
            exe_lba,
            lba_offset,
            track_base,
            checksum,
            loader_entry as *const () as u32,
            LoadingScreen::new(),
        )
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    runtime::panic(&mut LoadingScreen::new())
}
