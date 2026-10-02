static volatile unsigned diagnostic_count = 7;
static const unsigned limits[] = {12, 24, 48, 96};
__attribute__((section(".unusual_constants"), used))
const unsigned signature[] = {0x12345678, 0xabcdef01};
__attribute__((noinline))
unsigned diagnose(unsigned value) {
    volatile unsigned scratch[8];
    scratch[0] = limits[value & 3];
    diagnostic_count += scratch[0];
    return diagnostic_count;
}
