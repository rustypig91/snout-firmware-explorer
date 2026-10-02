ENTRY(Reset_Handler)
MEMORY {
  FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 256K
  RAM (rwx) : ORIGIN = 0x20000000, LENGTH = 64K
}
PHDRS {
  code PT_LOAD FLAGS(5);
  copied PT_LOAD FLAGS(5);
  data PT_LOAD FLAGS(6);
  zero PT_LOAD FLAGS(6);
}
SECTIONS {
  .text : { *(.text*) } > FLASH :code
  .rodata : { *(.rodata*) } > FLASH :code
  .unusual_constants : { *(.unusual_constants*) } > FLASH :code
  .ARM.exidx : { *(.ARM.exidx*) } > FLASH :code
  .ram_code : { *(.ram_code*) } > RAM AT> FLASH :copied
  .data : { *(.data*) } > RAM AT> FLASH :data
  .bss (NOLOAD) : { *(.bss*) *(COMMON) } > RAM :data
  .reserved (NOLOAD) : { . = ALIGN(8); . += 128; } > RAM :zero
  /DISCARD/ : { *(.comment) *(.note*) }
}
