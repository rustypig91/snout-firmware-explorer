/* Analysis fixture only: no vendor startup or runtime library. */
MEMORY
{
    FLASH (RX)  : origin = 0x08000000, length = 0x00040000
    RAM   (RWX) : origin = 0x20000000, length = 0x00010000
}

SECTIONS
{
    .text              : {} > FLASH
    .const             : {} > FLASH
    .rodata            : {} > FLASH
    .unusual_constants : {} > FLASH
    .ARM.exidx         : {} > FLASH
    .ARM.extab         : {} > FLASH
    .sensor_calibration_code : {} load = FLASH, run = RAM
    .ram_code          : {} load = FLASH, run = RAM
    .data              : {} load = FLASH, run = RAM
    .bss               : {} > RAM
    .reserved          : { . += 128; } > RAM, type = NOINIT, align = 8
}
