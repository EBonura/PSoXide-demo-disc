/*
 * Linker script template for the chain-load blob.
 *
 * build.rs fills in the two numbers from the `chainload-layout` crate, which is
 * where the launcher and mkdisc read them from too, and hands the result to the
 * linker. Do not link this file directly.
 *
 * The blob lives in the gap above the largest bottom-loaded program and below
 * the Half-Life intro, clear of the stack, so it survives while the target EXE
 * is streamed into RAM at the address its own header names (see
 * chainload-layout for the map).
 *
 * Output is a headerless raw binary: the launcher embeds these bytes,
 * copies them to LOADER_BASE, and jumps to offset 0.
 */

LOADER_BASE = @LOADER_BASE@;
LOADER_SIZE = @LOADER_BUDGET@;

ENTRY(loader_entry)

MEMORY {
    RAM (rwx) : ORIGIN = LOADER_BASE, LENGTH = LOADER_SIZE
}

SECTIONS {
    .text : {
        /* Entry must be at offset 0: the launcher jumps to LOADER_BASE. */
        KEEP(*(.text.loader_entry));
        *(.text .text.*);
        *(.rodata .rodata.*);
        *(.data .data.*);
        *(.got);
        . = ALIGN(4);
    } > RAM

    .bss (NOLOAD) : {
        __bss_start = .;
        *(.bss .bss.*);
        *(COMMON);
        . = ALIGN(4);
        __bss_end = .;
    } > RAM

    /* Nothing zeroes .bss for the blob (the launcher copies .text only),
       so any static here would be silently garbage. Keep the blob
       stack-only and let the link fail if that ever stops being true. */
    ASSERT(__bss_end == __bss_start, "loader blob must have no .bss")

    /* The last word of the reservation is the launcher's mailbox
       (chainload-layout::SHOWN_BUFFER_MAILBOX); the blob must stop short. */
    ASSERT(SIZEOF(.text) + 4 <= LOADER_SIZE, "loader blob overruns the mailbox word")

    /DISCARD/ : {
        *(.MIPS.abiflags)
        *(.reginfo)
        *(.eh_frame_hdr)
        *(.eh_frame)
        *(.comment)
        *(.note.*)
    }
}
