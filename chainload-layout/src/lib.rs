//! The demo disc's RAM map while a program is being chain-loaded.
//!
//! The launcher copies the loader blob to [`LOADER_BASE`], jumps to it, and the
//! blob streams the chosen program's payload into RAM at the address the
//! program's own header names. Three things must not overlap: the blob, the
//! stack it runs on, and that payload. The numbers live here and nowhere else:
//! the loader's linker script is generated from them, the launcher copies the
//! blob to them, and `mkdisc` refuses any program whose payload would touch
//! them.
//!
//! ```text
//! 0x80010000 .. up to LOADER_BASE   programs link here; the largest (Ocarina of
//!                                   Time) ends near 0x801B6000
//! LOADER_BASE .. +LOADER_BUDGET     the blob
//! .. 0x801F8000                     free; the Half-Life intro loads at its game's
//!                                   image end (0x801E0000 today) and ends at
//!                                   0x801F2800
//! STACK_BASE .. 0x80200000          the launcher's stack, shared with the blob
//! ```
//!
//! The blob sits in the gap between the largest bottom-loaded program and the
//! Half-Life intro. That intro cannot sit below the blob: it must stay above its
//! own game's image, which loads from 0x80010000.
#![no_std]

/// Where the blob runs. Page aligned.
pub const LOADER_BASE: u32 = 0x801C_8000;

/// Bytes reserved for the blob at [`LOADER_BASE`]. The launcher refuses a blob
/// over this at run time; the link fails over it too (`loader.ld`).
pub const LOADER_BUDGET: u32 = 32 * 1024;

/// Lowest address of the stack the launcher hands the blob.
pub const STACK_BASE: u32 = 0x801F_8000;

/// The word the launcher leaves for the blob: the VRAM row of the buffer the
/// display was showing when it handed over (0 or 240). It is the last word of
/// the blob's reservation, past the end of the blob (the link asserts the blob
/// stops short of it), so copying the blob does not touch it. The loader paints
/// its first frame into the other buffer and flips to it, so the launcher's
/// last picture stays up, whole, until the loader's first one is ready.
pub const SHOWN_BUFFER_MAILBOX: u32 = LOADER_BASE + LOADER_BUDGET - 4;

/// Top of RAM.
pub const RAM_END: u32 = 0x8020_0000;

/// Spans a payload must stay clear of: `(start, end)`, half open. The blob's
/// reservation and the stack.
pub const RESERVED: [(u32, u32); 2] = [
    (LOADER_BASE, LOADER_BASE + LOADER_BUDGET),
    (STACK_BASE, RAM_END),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reservations_are_ordered_and_inside_ram() {
        assert_eq!(LOADER_BASE % 0x1000, 0);
        assert!(LOADER_BASE >= 0x8001_0000);
        let [blob, stack] = RESERVED;
        assert!(blob.0 < blob.1 && blob.1 <= stack.0 && stack.0 < stack.1);
        assert!(SHOWN_BUFFER_MAILBOX >= blob.0 && SHOWN_BUFFER_MAILBOX + 4 == blob.1);
        assert_eq!(stack.1, RAM_END);
    }
}
